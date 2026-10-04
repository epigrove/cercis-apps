//! 本地 API：界面与内核之间的通道。
//!
//! 所有数据操作都经内核（cercis-core），界面自己不碰文件——
//! 这正是文档里的硬规则：**app 是用户与内核之间的唯一中介**。
//!
//! 语言：所有面向用户的文字走 `lang` 参数（zh/en），中英双语。

use cercis_core::query::{Op, Query, SortDir};
use cercis_core::Project;
use chrono::Datelike;
use serde_json::{json, Value};

use crate::http::{Request, Response};

/// 语言。
#[derive(Clone, Copy, PartialEq)]
pub enum Lang {
    Zh,
    En,
}

impl Lang {
    pub fn from_str(s: &str) -> Self {
        match s {
            "en" => Lang::En,
            _ => Lang::Zh,
        }
    }

    /// 取双语文本。
    pub fn t<'a>(&self, zh: &'a str, en: &'a str) -> &'a str {
        match self {
            Lang::Zh => zh,
            Lang::En => en,
        }
    }
}

/// 取字符串末尾若干字符（给界面显示插件输出用，避免超长文本）。
fn tail(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let start = text
        .char_indices()
        .rev()
        .nth(max)
        .map(|(i, _)| i)
        .unwrap_or(0);
    format!("…{}", &text[start..])
}

/// 处理一个 API 请求。
pub fn handle(req: &Request) -> Response {
    let lang = Lang::from_str(req.query.get("lang").map(|s| s.as_str()).unwrap_or("zh"));

    match req.path.as_str() {
        "/api/health" => Response::json(
            &json!({
                "ok": true,
                "core_version": cercis_core::CORE_VERSION,
                "plugin_api_version": cercis_core::PLUGIN_API_VERSION,
            })
            .to_string(),
        ),

        // 打开项目：返回概览（表、行数、模板、语言）
        "/api/open" => {
            let Some(path) = req.query.get("path") else {
                return Response::error(400, lang.t("缺少 path 参数", "missing path parameter"));
            };
            match Project::open(path, None) {
                Ok(p) => {
                    let template = p
                        .template()
                        .unwrap_or_else(|_| cercis_core::Template::generic());
                    // 多人项目：把"要不要登录"告诉界面（有账户但没登录时写入会被拒）
                    let account_required = cercis_core::accounts::Accounts::load(p.root())
                        .map(|a| a.enabled())
                        .unwrap_or(false);
                    let whoami = cercis_core::accounts::current(p.root())
                        .ok()
                        .flatten()
                        .map(|s| {
                            json!({
                                "id": s.id,
                                "role": s.role,
                                "expires": s.expires,
                            })
                        });
                    let mut tables = Vec::new();
                    for t in &template.tables {
                        // 走内核的存储层：加密区域里的表在磁盘上是密文，
                        // 这里会自动透明解密；没解锁就报 0 行而不是崩掉。
                        let (rows, cols, locked) = match p.store.load_table(&t.name, &[]) {
                            Ok(tb) => (tb.rows.len(), tb.columns.clone(), false),
                            Err(e) => {
                                let locked = e.to_string().contains("加密区域");
                                (0, vec![], locked)
                            }
                        };
                        tables.push(json!({
                            "name": t.name,
                            "label": if lang == Lang::Zh { &t.label_cn } else { &t.label_en },
                            "primary_key": t.primary_key,
                            "rows": rows,
                            "columns": cols,
                            "locked": locked,
                        }));
                    }
                    Response::json(
                        &json!({
                            "ok": true,
                            "project": p.config.name,
                            "root": p.root().display().to_string(),
                            "template": template.name,
                            "language": if lang == Lang::Zh { "zh" } else { "en" },
                            "account_required": account_required,
                            "whoami": whoami,
                            "tables": tables,
                        })
                        .to_string(),
                    )
                }
                Err(e) => Response::error(404, &e.to_string()),
            }
        }

        // 查询表数据
        "/api/table" => {
            let Some(path) = req.query.get("path") else {
                return Response::error(400, lang.t("缺少 path 参数", "missing path parameter"));
            };
            let Some(table) = req.query.get("table") else {
                return Response::error(400, lang.t("缺少 table 参数", "missing table parameter"));
            };
            let limit = req
                .query
                .get("limit")
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(200)
                .min(5000);
            // 分页：大表不一次全丢给浏览器（渲染卡顿主要来自 DOM，不是读 CSV）
            let offset = req
                .query
                .get("offset")
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(0);

            match Project::open(path, None) {
                Ok(p) => {
                    // 先不带分页跑一遍拿总数（筛选后的总数才是用户想看的）
                    let mut q = Query::new();
                    // 可选排序
                    if let Some(sort) = req.query.get("sort") {
                        let (col, dir) = match sort.split_once(':') {
                            Some((c, "desc")) => (c, SortDir::Desc),
                            Some((c, _)) => (c, SortDir::Asc),
                            None => (sort.as_str(), SortDir::Asc),
                        };
                        q = q.sort(col, dir);
                    }
                    // 可选筛选：filter=col:op:value
                    if let Some(f) = req.query.get("filter") {
                        let parts: Vec<&str> = f.splitn(3, ':').collect();
                        if parts.len() == 3 {
                            let op = match parts[1] {
                                "eq" => Op::Eq,
                                "ne" => Op::Ne,
                                "lt" => Op::Lt,
                                "le" => Op::Le,
                                "gt" => Op::Gt,
                                "ge" => Op::Ge,
                                "contains" => Op::Contains,
                                "startswith" => Op::StartsWith,
                                "in" => Op::In,
                                "isempty" => Op::IsEmpty,
                                "notempty" => Op::NotEmpty,
                                _ => Op::Eq,
                            };
                            q = q.filter(parts[0], op, parts[2]);
                        }
                    }

                    let mut q_all = q.clone();
                    q = q.page(offset, limit);
                    match (p.query(table, q), p.query(table, q_all.clone())) {
                        (Ok(rows), Ok(all)) => {
                            let total = all.len();
                            let table_def = p.template().ok().and_then(|t| t.table(table).cloned());
                            let columns = table_def
                                .as_ref()
                                .map(|t| {
                                    t.fields
                                        .iter()
                                        .map(|f| {
                                            json!({
                                                "name": f.name,
                                                "label": if lang == Lang::Zh { &f.label_cn } else { &f.label_en },
                                                "type": f.field_type,
                                                "unit": f.unit,
                                            })
                                        })
                                        .collect::<Vec<_>>()
                                })
                                .unwrap_or_default();
                            Response::json(
                                &json!({
                                    "ok": true,
                                    "table": table,
                                    "columns": columns,
                                    "count": rows.len(),
                                    "total": total,
                                    "offset": offset,
                                    "limit": limit,
                                    "rows": rows,
                                })
                                .to_string(),
                            )
                        }
                        (Err(e), _) | (_, Err(e)) => Response::error(400, &e.to_string()),
                    }
                }
                Err(e) => Response::error(404, &e.to_string()),
            }
        }

        // 校验项目
        "/api/validate" => {
            let Some(path) = req.query.get("path") else {
                return Response::error(400, lang.t("缺少 path 参数", "missing path parameter"));
            };
            match Project::open(path, None) {
                Ok(p) => match p.validate() {
                    Ok(r) => Response::json(
                        &json!({
                            "ok": r.is_ok(),
                            "errors": r.errors,
                            "warnings": r.warnings,
                        })
                        .to_string(),
                    ),
                    Err(e) => Response::error(500, &e.to_string()),
                },
                Err(e) => Response::error(404, &e.to_string()),
            }
        }

        // 变更流水
        "/api/log" => {
            let Some(path) = req.query.get("path") else {
                return Response::error(400, lang.t("缺少 path 参数", "missing path parameter"));
            };
            let limit = req
                .query
                .get("limit")
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(50);
            match Project::open(path, None) {
                Ok(p) => match p.store.read_change_log() {
                    Ok(entries) => {
                        let list: Vec<_> = entries.iter().rev().take(limit).collect();
                        Response::json(&json!({ "ok": true, "entries": list }).to_string())
                    }
                    Err(e) => Response::error(500, &e.to_string()),
                },
                Err(e) => Response::error(404, &e.to_string()),
            }
        }

        // 加密区域状态（S11）：区域、成员、有没有明文残留、当前解锁身份
        "/api/access" => {
            let Some(path) = req.query.get("path") else {
                return Response::error(400, lang.t("缺少 path 参数", "missing path parameter"));
            };
            let root = std::path::Path::new(path);
            let member = req.query.get("member").map(|s| s.as_str());
            match cercis_core::access::status(root, member) {
                Ok(st) => {
                    let issues = cercis_core::access::doctor_issues(root).unwrap_or_default();
                    Response::json(
                        &json!({
                            "ok": true,
                            "members": st.members.iter().map(|m| json!({
                                "id": m.id,
                                "display_name": m.display_name,
                            })).collect::<Vec<_>>(),
                            "unlocked_as": st.session_member,
                            "unlocked_members": cercis_core::access::unlocked_members(root),
                            "keychain_available": st.keychain_available,
                            "share_home": st.share_home,
                            "zones": st.zones.iter().map(|z| json!({
                                "zone": z.zone,
                                "label": z.label,
                                "generation": z.generation,
                                "key_fingerprint": z.key_fingerprint,
                                "files": z.files,
                                "bytes": z.bytes,
                                "permission": z.permission.map(|p| p.as_str()),
                                "plaintext_left": z.plaintext_left,
                                "unreadable": z.unreadable,
                                "members": z.members.iter().map(|(id, p)| json!({
                                    "id": id, "permission": p.as_str(),
                                })).collect::<Vec<_>>(),
                            })).collect::<Vec<_>>(),
                            "issues": issues,
                        })
                        .to_string(),
                    )
                }
                Err(e) => Response::error(400, &e.to_string()),
            }
        }

        // 解锁（S11）：口令由用户在界面上输入；app **不留存**口令，也不写进项目
        "/api/access/unlock" => {
            if req.method != "POST" {
                return Response::error(400, "POST only");
            }
            let body: Value = match serde_json::from_str(&req.body) {
                Ok(v) => v,
                Err(e) => return Response::error(400, &format!("body 不是 JSON：{e}")),
            };
            let path = body.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let member = body.get("member").and_then(|v| v.as_str()).unwrap_or("");
            let password = body.get("password").and_then(|v| v.as_str()).unwrap_or("");
            if path.is_empty() || member.is_empty() || password.is_empty() {
                return Response::error(
                    400,
                    lang.t(
                        "需要 path、member 与 password",
                        "path, member and password are required",
                    ),
                );
            }
            let ttl = body
                .get("ttl_hours")
                .and_then(|v| v.as_i64())
                .unwrap_or(12);
            let remember = body
                .get("remember")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            match cercis_core::access::unlock(
                std::path::Path::new(path),
                member,
                password,
                ttl,
                remember,
            ) {
                Ok(rep) => Response::json(
                    &json!({
                        "ok": true,
                        "member": rep.member,
                        "expires": rep.expires,
                        "zones": rep.zones.iter().map(|z| json!({
                            "name": z.name,
                            "permission": z.permission.as_str(),
                            "key_fingerprint": z.key_fingerprint,
                        })).collect::<Vec<_>>(),
                        "notes": rep.notes,
                    })
                    .to_string(),
                ),
                Err(e) => Response::error(400, &e.to_string()),
            }
        }

        // 上锁（S11）：清掉本机会话（可选连钥匙串里的口令一起删）
        "/api/access/lock" => {
            if req.method != "POST" {
                return Response::error(400, "POST only");
            }
            let body: Value = match serde_json::from_str(&req.body) {
                Ok(v) => v,
                Err(e) => return Response::error(400, &format!("body 不是 JSON：{e}")),
            };
            let path = body.get("path").and_then(|v| v.as_str()).unwrap_or("");
            if path.is_empty() {
                return Response::error(400, lang.t("缺少 path", "missing path"));
            }
            let member = body.get("member").and_then(|v| v.as_str());
            let forget = body
                .get("forget")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            match cercis_core::access::lock(std::path::Path::new(path), member, forget) {
                Ok(locked) => {
                    Response::json(&json!({ "ok": true, "locked": locked }).to_string())
                }
                Err(e) => Response::error(400, &e.to_string()),
            }
        }

        // 搬运：打包带走（S12）——全量/增量/分卷，加密区域保持密文
        "/api/pack" => {
            if req.method != "POST" {
                return Response::error(400, "POST only");
            }
            let body: Value = match serde_json::from_str(&req.body) {
                Ok(v) => v,
                Err(e) => return Response::error(400, &format!("body 不是 JSON：{e}")),
            };
            let path = body.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let out = body.get("out").and_then(|v| v.as_str()).unwrap_or("");
            if path.is_empty() || out.is_empty() {
                return Response::error(
                    400,
                    lang.t("需要 path 与 out", "path and out are required"),
                );
            }
            let opts = cercis_core::PackOptions {
                part_size: body
                    .get("part_size_mb")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(20)
                    * 1024
                    * 1024,
                since: body
                    .get("since")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                    .map(std::path::PathBuf::from),
                include_history: !body
                    .get("no_history")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
                with_checkpoints: body
                    .get("with_checkpoints")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
                dry_run: body
                    .get("dry_run")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
            };
            let out_path = std::path::PathBuf::from(out);
            if !opts.dry_run {
                if let Err(e) = std::fs::create_dir_all(&out_path) {
                    return Response::error(400, &format!("建不了输出目录：{e}"));
                }
            }
            match cercis_core::pack::pack(std::path::Path::new(path), &out_path, &opts) {
                Ok(rep) => Response::json(
                    &json!({
                        "ok": true,
                        "dry_run": rep.dry_run,
                        "out": rep.out,
                        "parts": rep.parts,
                        "files": rep.files,
                        "bytes": rep.bytes,
                        "skipped_unchanged": rep.skipped_unchanged,
                        "removed": rep.removed,
                        "notes": rep.notes,
                        "manifest": rep.manifest,
                    })
                    .to_string(),
                ),
                Err(e) => Response::error(400, &e.to_string()),
            }
        }

        // 搬运：合并一个包（S12）——先校验，再并集；冲突留双方副本
        "/api/merge" => {
            if req.method != "POST" {
                return Response::error(400, "POST only");
            }
            let body: Value = match serde_json::from_str(&req.body) {
                Ok(v) => v,
                Err(e) => return Response::error(400, &format!("body 不是 JSON：{e}")),
            };
            let pack = body.get("pack").and_then(|v| v.as_str()).unwrap_or("");
            let into = body.get("into").and_then(|v| v.as_str()).unwrap_or("");
            if pack.is_empty() || into.is_empty() {
                return Response::error(
                    400,
                    lang.t("需要 pack 与 into", "pack and into are required"),
                );
            }
            let opts = cercis_core::pack::MergeOptions {
                dry_run: body
                    .get("dry_run")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
                member: body
                    .get("member")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                    .map(|s| s.to_string()),
            };
            match cercis_core::pack::merge(
                std::path::Path::new(pack),
                std::path::Path::new(into),
                &opts,
            ) {
                Ok(rep) => Response::json(
                    &json!({
                        "ok": true,
                        "dry_run": rep.dry_run,
                        "summary": rep.summary(),
                        "verified": rep.verified,
                        "added": rep.added,
                        "merged": rep.merged,
                        "conflicts": rep.conflicts,
                        "unchanged": rep.unchanged,
                        "refused": rep.refused,
                        "removed_reported": rep.removed_reported,
                        "report_path": rep.report_path,
                        "notes": rep.notes,
                        "project_name": rep.project_name,
                    })
                    .to_string(),
                ),
                Err(e) => Response::error(400, &e.to_string()),
            }
        }

        // 搬运：列合并报告（项目 _labnote/ 下的 merge-report-*.md/.json）
        // 共存体检（S13 / FR-I06）：云盘 / Git 目录里的分叉风险
        "/api/sync" => {
            let Some(path) = req.query.get("path") else {
                return Response::error(400, lang.t("缺少 path 参数", "missing path parameter"));
            };
            match cercis_core::sync::check(std::path::Path::new(path)) {
                Ok(rep) => Response::json(
                    &json!({
                        "ok": rep.verdict != "needs-merge",
                        "verdict": rep.verdict,
                        "cloud": rep.cloud,
                        "git": rep.git,
                        "findings": rep.findings,
                        "notes": rep.notes,
                        "summary": rep.summary(),
                    })
                    .to_string(),
                ),
                Err(e) => Response::error(400, &e.to_string()),
            }
        }

        // 搬运：列合并报告（项目 _labnote/ 下的 merge-report-*.md/.json）
        "/api/reports" => {
            let Some(path) = req.query.get("path") else {
                return Response::error(400, lang.t("缺少 path 参数", "missing path parameter"));
            };
            let dir = std::path::Path::new(path).join("_labnote");
            let mut out = Vec::new();
            if let Ok(rd) = std::fs::read_dir(&dir) {
                for e in rd.flatten() {
                    let name = e.file_name().to_string_lossy().to_string();
                    if !name.starts_with("merge-report-") || !name.ends_with(".md") {
                        continue;
                    }
                    let meta = e.metadata().ok();
                    let mtime_unix = meta
                        .as_ref()
                        .and_then(|m| m.modified().ok())
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    out.push(json!({
                        "name": name,
                        "bytes": meta.as_ref().map(|m| m.len()).unwrap_or(0),
                        "mtime_unix": mtime_unix,
                    }));
                }
            }
            out.sort_by(|a, b| b["name"].as_str().cmp(&a["name"].as_str()));
            Response::json(&json!({ "ok": true, "reports": out }).to_string())
        }

        // 搬运：读一份报告（只允许读项目 _labnote/ 下的 md/json）
        "/api/report" => {
            let Some(path) = req.query.get("path") else {
                return Response::error(400, lang.t("缺少 path 参数", "missing path parameter"));
            };
            let Some(file) = req.query.get("file") else {
                return Response::error(400, lang.t("缺少 file 参数", "missing file parameter"));
            };
            // 防目录穿越：只认文件名，且必须是 md/json
            if file.contains('/') || file.contains("..") || !(file.ends_with(".md") || file.ends_with(".json")) {
                return Response::error(400, lang.t("只允许读 _labnote 下的报告", "only reports are allowed"));
            }
            let p = std::path::Path::new(path).join("_labnote").join(file);
            match std::fs::read_to_string(&p) {
                Ok(text) => Response::json(
                    &json!({ "ok": true, "file": file, "text": text }).to_string(),
                ),
                Err(e) => Response::error(404, &format!("读不了：{e}")),
            }
        }

        // ———— 待办（tables/tasks.csv；§5.2.11：内核不解释语义，界面负责"今天/本周/逾期"）————
        "/api/todos" => {
            let Some(path) = req.query.get("path") else {
                return Response::error(400, lang.t("缺少 path 参数", "missing path parameter"));
            };
            match read_tasks(path) {
                Ok((exists, tasks)) => Response::json(
                    &json!({ "ok": true, "exists": exists, "tasks": tasks }).to_string(),
                ),
                Err(e) => Response::error(400, &e.to_string()),
            }
        }

        // 建一条待办（没有 tasks.csv 就按标准列建）
        "/api/todos/add" => {
            if req.method != "POST" {
                return Response::error(400, "POST only");
            }
            let body: Value = match serde_json::from_str(&req.body) {
                Ok(v) => v,
                Err(e) => return Response::error(400, &format!("body 不是 JSON：{e}")),
            };
            let path = body.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let title = body.get("title").and_then(|v| v.as_str()).unwrap_or("");
            if path.is_empty() || title.trim().is_empty() {
                return Response::error(
                    400,
                    lang.t("需要 path 与 title", "path and title are required"),
                );
            }
            match add_task(path, &body) {
                Ok(id) => Response::json(&json!({ "ok": true, "task_id": id }).to_string()),
                Err(e) => Response::error(400, &e.to_string()),
            }
        }

        // 勾选完成 / 取消完成：既改当前行（前值进历史对象库），又写一条更正记录
        "/api/todos/done" => {
            if req.method != "POST" {
                return Response::error(400, "POST only");
            }
            let body: Value = match serde_json::from_str(&req.body) {
                Ok(v) => v,
                Err(e) => return Response::error(400, &format!("body 不是 JSON：{e}")),
            };
            let path = body.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let task_id = body.get("task_id").and_then(|v| v.as_str()).unwrap_or("");
            let done = body.get("done").and_then(|v| v.as_bool()).unwrap_or(true);
            if path.is_empty() || task_id.is_empty() {
                return Response::error(
                    400,
                    lang.t("需要 path 与 task_id", "path and task_id are required"),
                );
            }
            match set_task_done(path, task_id, done) {
                Ok((old, new)) => Response::json(
                    &json!({ "ok": true, "task_id": task_id, "old": old, "new": new }).to_string(),
                ),
                Err(e) => Response::error(400, &e.to_string()),
            }
        }

        // ———— 历史审阅（FR-M02/M05/M07）：流水按表/人/设备筛，单条记录看版本链 ————
        "/api/history" => {
            let Some(path) = req.query.get("path") else {
                return Response::error(400, lang.t("缺少 path 参数", "missing path parameter"));
            };
            let table = req.query.get("table").cloned().unwrap_or_default();
            let user = req.query.get("user").cloned().unwrap_or_default();
            let device = req.query.get("device").cloned().unwrap_or_default();
            let id = req.query.get("id").cloned().unwrap_or_default();
            let limit = req
                .query
                .get("limit")
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(100);
            match Project::open(path, None) {
                Ok(p) => match p.store.read_change_log() {
                    Ok(entries) => {
                        let mut list: Vec<_> = entries
                            .into_iter()
                            .rev()
                            .filter(|e| table.is_empty() || e.target_table == table)
                            .filter(|e| user.is_empty() || e.user_id == user)
                            .filter(|e| device.is_empty() || e.device_id == device)
                            .filter(|e| id.is_empty() || e.target_id == id)
                            .take(limit)
                            .collect();
                        // 顺手告诉界面"这台设备上有哪些人"（筛选用）
                        list.shrink_to_fit();
                        Response::json(
                            &json!({ "ok": true, "count": list.len(), "entries": list })
                                .to_string(),
                        )
                    }
                    Err(e) => Response::error(500, &e.to_string()),
                },
                Err(e) => Response::error(404, &e.to_string()),
            }
        }

        // 一条记录的完整版本链：更正记录 + 相关流水
        "/api/history/record" => {
            let Some(path) = req.query.get("path") else {
                return Response::error(400, lang.t("缺少 path 参数", "missing path parameter"));
            };
            let table = req.query.get("table").cloned().unwrap_or_default();
            let id = req.query.get("id").cloned().unwrap_or_default();
            if table.is_empty() || id.is_empty() {
                return Response::error(
                    400,
                    lang.t("需要 table 与 id", "table and id are required"),
                );
            }
            match Project::open(path, None) {
                Ok(p) => {
                    // 更正：只挑这一条记录的
                    let corrections: Vec<Value> = p
                        .store
                        .load_table("corrections", &[])
                        .ok()
                        .map(|t| {
                            t.rows
                                .iter()
                                .filter(|r| {
                                    r.get("target_table").map(|s| s.as_str()) == Some(&table)
                                        && r.get("target_id").map(|s| s.as_str()) == Some(&id)
                                })
                                .map(|r| {
                                    json!({
                                        "correction_id": r.get("correction_id"),
                                        "field": r.get("field"),
                                        "old": r.get("old_value"),
                                        "new": r.get("new_value"),
                                        "reason": r.get("reason"),
                                        "user": r.get("user_id"),
                                        "ts": r.get("ts"),
                                    })
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    // 流水：这一条记录上发生过的写入
                    let entries: Vec<Value> = p
                        .store
                        .read_change_log()
                        .unwrap_or_default()
                        .into_iter()
                        .filter(|e| e.target_table == table && e.target_id == id)
                        .map(|e| {
                            json!({
                                "change_id": e.change_id,
                                "ts": e.ts,
                                "action": e.action,
                                "summary": e.summary,
                                "user": e.user_id,
                                "device": e.device_id,
                                "hash_before": e.hash_before,
                                "hash_after": e.hash_after,
                            })
                        })
                        .collect();
                    Response::json(
                        &json!({
                            "ok": true, "table": table, "id": id,
                            "corrections": corrections, "entries": entries,
                        })
                        .to_string(),
                    )
                }
                Err(e) => Response::error(404, &e.to_string()),
            }
        }

        // ———— NAS（FR-N01–N07）：网络位置、体检、写租约、工作副本 ————
        "/api/remote" => {
            let Some(path) = req.query.get("path") else {
                return Response::error(400, lang.t("缺少 path 参数", "missing path parameter"));
            };
            let root = std::path::Path::new(path);
            let remotes = cercis_core::remote::list(root).unwrap_or_default();
            let leases = cercis_core::remote::active_leases(root).unwrap_or_default();
            Response::json(
                &json!({
                    "ok": true,
                    "remotes": remotes.iter().map(|r| json!({
                        "name": r.name,
                        "kind": r.kind,
                        "path": r.path,
                        "note": r.note,
                        "reachable": std::path::Path::new(&r.path).exists(),
                        "credential": !r.credential_account.is_empty(),
                    })).collect::<Vec<_>>(),
                    "leases": leases,
                })
                .to_string(),
            )
        }

        "/api/remote/add" => {
            if req.method != "POST" {
                return Response::error(400, "POST only");
            }
            let body: Value = match serde_json::from_str(&req.body) {
                Ok(v) => v,
                Err(e) => return Response::error(400, &format!("body 不是 JSON：{e}")),
            };
            let path = body.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let name = body.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let remote_path = body
                .get("remote_path")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let kind = body.get("kind").and_then(|v| v.as_str()).unwrap_or("");
            let note = body.get("note").and_then(|v| v.as_str()).unwrap_or("");
            match cercis_core::remote::add(
                std::path::Path::new(path),
                name,
                kind,
                remote_path,
                note,
            ) {
                Ok(r) => Response::json(
                    &json!({ "ok": true, "name": r.name, "path": r.path, "kind": r.kind })
                        .to_string(),
                ),
                Err(e) => Response::error(400, &e.to_string()),
            }
        }

        "/api/remote/remove" => {
            if req.method != "POST" {
                return Response::error(400, "POST only");
            }
            let body: Value = match serde_json::from_str(&req.body) {
                Ok(v) => v,
                Err(e) => return Response::error(400, &format!("body 不是 JSON：{e}")),
            };
            let path = body.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let name = body.get("name").and_then(|v| v.as_str()).unwrap_or("");
            match cercis_core::remote::remove(std::path::Path::new(path), name) {
                Ok(r) => Response::json(&json!({ "ok": true, "removed": r.name }).to_string()),
                Err(e) => Response::error(400, &e.to_string()),
            }
        }

        "/api/remote/test" => {
            let Some(path) = req.query.get("path") else {
                return Response::error(400, lang.t("缺少 path 参数", "missing path parameter"));
            };
            let name = req.query.get("name").map(|s| s.as_str()).unwrap_or("");
            match cercis_core::remote::test(std::path::Path::new(path), name) {
                Ok(t) => Response::json(
                    &json!({
                        "ok": t.reachable && t.readable,
                        "summary": t.summary(),
                        "test": t,
                    })
                    .to_string(),
                ),
                Err(e) => Response::error(400, &e.to_string()),
            }
        }

        "/api/remote/claim" => {
            if req.method != "POST" {
                return Response::error(400, "POST only");
            }
            let body: Value = match serde_json::from_str(&req.body) {
                Ok(v) => v,
                Err(e) => return Response::error(400, &format!("body 不是 JSON：{e}")),
            };
            let path = body.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let who = body.get("who").and_then(|v| v.as_str()).unwrap_or("");
            let minutes = body
                .get("minutes")
                .and_then(|v| v.as_i64())
                .unwrap_or(30);
            match cercis_core::remote::claim(std::path::Path::new(path), who, minutes) {
                Ok(msg) => Response::json(&json!({ "ok": true, "message": msg }).to_string()),
                Err(e) => Response::error(400, &e.to_string()),
            }
        }

        "/api/remote/release" => {
            if req.method != "POST" {
                return Response::error(400, "POST only");
            }
            let body: Value = match serde_json::from_str(&req.body) {
                Ok(v) => v,
                Err(e) => return Response::error(400, &format!("body 不是 JSON：{e}")),
            };
            let path = body.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let who = body.get("who").and_then(|v| v.as_str()).unwrap_or("");
            match cercis_core::remote::release(std::path::Path::new(path), who) {
                Ok(n) => Response::json(&json!({ "ok": true, "cleared": n }).to_string()),
                Err(e) => Response::error(400, &e.to_string()),
            }
        }

        // 远程凭据：只进系统钥匙串（口令只在这一次请求里用）
        "/api/remote/credential" => {
            if req.method != "POST" {
                return Response::error(400, "POST only");
            }
            let body: Value = match serde_json::from_str(&req.body) {
                Ok(v) => v,
                Err(e) => return Response::error(400, &format!("body 不是 JSON：{e}")),
            };
            let path = body.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let name = body.get("name").and_then(|v| v.as_str()).unwrap_or("");
            if let Some(secret) = body.get("password").and_then(|v| v.as_str()) {
                match cercis_core::remote::credential_set(std::path::Path::new(path), name, secret) {
                    Ok(account) => Response::json(
                        &json!({ "ok": true, "keychain_account": account }).to_string(),
                    ),
                    Err(e) => Response::error(400, &e.to_string()),
                }
            } else {
                match cercis_core::remote::credential_delete(std::path::Path::new(path), name) {
                    Ok(removed) => {
                        Response::json(&json!({ "ok": true, "removed": removed }).to_string())
                    }
                    Err(e) => Response::error(400, &e.to_string()),
                }
            }
        }

        // 工作副本：状态 / 建 / 双向同步（FR-N03 / FR-N07）
        "/api/workcopy" => {
            let Some(path) = req.query.get("path") else {
                return Response::error(400, lang.t("缺少 path 参数", "missing path parameter"));
            };
            let root = std::path::Path::new(path);
            if !root.join("_labnote/workcopy.yaml").exists() {
                return Response::json(
                    &json!({ "ok": true, "is_workcopy": false }).to_string(),
                );
            }
            match cercis_core::remote::workcopy_status(root) {
                Ok(st) => Response::json(
                    &json!({
                        "ok": true, "is_workcopy": true,
                        "summary": st.summary(), "status": st,
                    })
                    .to_string(),
                ),
                Err(e) => Response::error(400, &e.to_string()),
            }
        }

        "/api/workcopy/create" => {
            if req.method != "POST" {
                return Response::error(400, "POST only");
            }
            let body: Value = match serde_json::from_str(&req.body) {
                Ok(v) => v,
                Err(e) => return Response::error(400, &format!("body 不是 JSON：{e}")),
            };
            let main = body.get("main").and_then(|v| v.as_str()).unwrap_or("");
            let to = body.get("to").and_then(|v| v.as_str()).unwrap_or("");
            let name = body.get("name").and_then(|v| v.as_str()).unwrap_or("");
            if main.is_empty() || to.is_empty() {
                return Response::error(
                    400,
                    lang.t("需要 main 与 to", "main and to are required"),
                );
            }
            match cercis_core::remote::workcopy_create(
                std::path::Path::new(main),
                std::path::Path::new(to),
                name,
            ) {
                Ok(wc) => Response::json(
                    &json!({ "ok": true, "main": wc.main, "to": to }).to_string(),
                ),
                Err(e) => Response::error(400, &e.to_string()),
            }
        }

        "/api/workcopy/sync" => {
            if req.method != "POST" {
                return Response::error(400, "POST only");
            }
            let body: Value = match serde_json::from_str(&req.body) {
                Ok(v) => v,
                Err(e) => return Response::error(400, &format!("body 不是 JSON：{e}")),
            };
            let copy = body.get("copy").and_then(|v| v.as_str()).unwrap_or("");
            let dry_run = body
                .get("dry_run")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if copy.is_empty() {
                return Response::error(400, lang.t("缺少 copy", "missing copy"));
            }
            match cercis_core::remote::workcopy_sync(std::path::Path::new(copy), dry_run) {
                Ok(rep) => Response::json(
                    &json!({
                        "ok": true,
                        "dry_run": rep.dry_run,
                        "summary": rep.summary(),
                        "conflicts_total": rep.conflicts_total,
                        "pulled_added": rep.pulled.added,
                        "pushed_added": rep.pushed.added,
                        "notes": rep.notes,
                    })
                    .to_string(),
                ),
                Err(e) => Response::error(400, &e.to_string()),
            }
        }

        // 加密区域管理：建成员 / 建区域 / 授权 / 移除（S13 表单用；口令只在一次请求里用）
        // 多人账户（FR-N08/N09）：看、登录、登出、建账户、改角色、删账户
        "/api/account" => {
            let Some(path) = req.query.get("path") else {
                return Response::error(400, lang.t("缺少 path 参数", "missing path parameter"));
            };
            let root = std::path::Path::new(path);
            let accounts = cercis_core::accounts::list(root).unwrap_or_default();
            let session = cercis_core::accounts::current(root).ok().flatten();
            Response::json(
                &json!({
                    "ok": true,
                    "enabled": !accounts.is_empty(),
                    "whoami": session.map(|s| json!({
                        "id": s.id, "role": s.role, "expires": s.expires,
                    })),
                    "accounts": accounts.iter().map(|a| json!({
                        "id": a.id,
                        "display_name": a.display_name,
                        "role": a.role,
                        "note": a.note,
                    })).collect::<Vec<_>>(),
                    "keychain_available": cercis_core::keychain::available(),
                })
                .to_string(),
            )
        }

        "/api/account/login" => {
            if req.method != "POST" {
                return Response::error(400, "POST only");
            }
            let body: Value = match serde_json::from_str(&req.body) {
                Ok(v) => v,
                Err(e) => return Response::error(400, &format!("body 不是 JSON：{e}")),
            };
            let path = body.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let id = body.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let password = body.get("password").and_then(|v| v.as_str()).unwrap_or("");
            if path.is_empty() || id.is_empty() || password.is_empty() {
                return Response::error(
                    400,
                    lang.t("需要 path、id 与口令", "path, id and password are required"),
                );
            }
            let ttl = body
                .get("ttl_hours")
                .and_then(|v| v.as_i64())
                .unwrap_or(12);
            match cercis_core::accounts::login(
                std::path::Path::new(path),
                id,
                password,
                cercis_core::crypto::KdfParams::from_env(),
                ttl,
            ) {
                Ok(s) => Response::json(
                    &json!({
                        "ok": true, "id": s.id, "role": s.role, "expires": s.expires,
                    })
                    .to_string(),
                ),
                Err(e) => Response::error(400, &e.to_string()),
            }
        }

        "/api/account/logout" => {
            if req.method != "POST" {
                return Response::error(400, "POST only");
            }
            let body: Value = match serde_json::from_str(&req.body) {
                Ok(v) => v,
                Err(e) => return Response::error(400, &format!("body 不是 JSON：{e}")),
            };
            let path = body.get("path").and_then(|v| v.as_str()).unwrap_or("");
            if path.is_empty() {
                return Response::error(400, lang.t("缺少 path", "missing path"));
            }
            match cercis_core::accounts::logout(std::path::Path::new(path)) {
                Ok(removed) => {
                    Response::json(&json!({ "ok": true, "logged_out": removed }).to_string())
                }
                Err(e) => Response::error(400, &e.to_string()),
            }
        }

        "/api/account/add" => {
            if req.method != "POST" {
                return Response::error(400, "POST only");
            }
            let body: Value = match serde_json::from_str(&req.body) {
                Ok(v) => v,
                Err(e) => return Response::error(400, &format!("body 不是 JSON：{e}")),
            };
            let path = body.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let id = body.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let name = body.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let role = body.get("role").and_then(|v| v.as_str()).unwrap_or("editor");
            let password = body.get("password").and_then(|v| v.as_str()).unwrap_or("");
            if path.is_empty() || id.is_empty() || password.is_empty() {
                return Response::error(
                    400,
                    lang.t("需要 path、id 与口令", "path, id and password are required"),
                );
            }
            let role = match cercis_core::Role::parse(role) {
                Ok(r) => r,
                Err(e) => return Response::error(400, &e.to_string()),
            };
            match cercis_core::accounts::add(
                std::path::Path::new(path),
                id,
                name,
                role,
                password,
                cercis_core::crypto::KdfParams::from_env(),
                "",
            ) {
                Ok(a) => Response::json(
                    &json!({ "ok": true, "id": a.id, "role": a.role }).to_string(),
                ),
                Err(e) => Response::error(400, &e.to_string()),
            }
        }

        "/api/account/role" => {
            if req.method != "POST" {
                return Response::error(400, "POST only");
            }
            let body: Value = match serde_json::from_str(&req.body) {
                Ok(v) => v,
                Err(e) => return Response::error(400, &format!("body 不是 JSON：{e}")),
            };
            let path = body.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let id = body.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let role = body.get("role").and_then(|v| v.as_str()).unwrap_or("viewer");
            let actor = body
                .get("as")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty());
            if path.is_empty() || id.is_empty() {
                return Response::error(400, lang.t("需要 path 与 id", "path and id are required"));
            }
            let role = match cercis_core::Role::parse(role) {
                Ok(r) => r,
                Err(e) => return Response::error(400, &e.to_string()),
            };
            match cercis_core::accounts::set_role(std::path::Path::new(path), id, role, actor) {
                Ok(a) => Response::json(
                    &json!({ "ok": true, "id": a.id, "role": a.role }).to_string(),
                ),
                Err(e) => Response::error(400, &e.to_string()),
            }
        }

        "/api/account/remove" => {
            if req.method != "POST" {
                return Response::error(400, "POST only");
            }
            let body: Value = match serde_json::from_str(&req.body) {
                Ok(v) => v,
                Err(e) => return Response::error(400, &format!("body 不是 JSON：{e}")),
            };
            let path = body.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let id = body.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let actor = body
                .get("as")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty());
            if path.is_empty() || id.is_empty() {
                return Response::error(400, lang.t("需要 path 与 id", "path and id are required"));
            }
            match cercis_core::accounts::remove(std::path::Path::new(path), id, actor) {
                Ok(()) => Response::json(&json!({ "ok": true, "removed": id }).to_string()),
                Err(e) => Response::error(400, &e.to_string()),
            }
        }

        "/api/access/member" => {
            if req.method != "POST" {
                return Response::error(400, "POST only");
            }
            let body: Value = match serde_json::from_str(&req.body) {
                Ok(v) => v,
                Err(e) => return Response::error(400, &format!("body 不是 JSON：{e}")),
            };
            let path = body.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let id = body.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let name = body.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let password = body.get("password").and_then(|v| v.as_str()).unwrap_or("");
            let note = body.get("note").and_then(|v| v.as_str()).unwrap_or("");
            if path.is_empty() || id.is_empty() || password.is_empty() {
                return Response::error(
                    400,
                    lang.t(
                        "需要 path、id 与 password",
                        "path, id and password are required",
                    ),
                );
            }
            match cercis_core::access::create_member(
                std::path::Path::new(path),
                id,
                name,
                password,
                cercis_core::crypto::KdfParams::from_env(),
                note,
            ) {
                Ok(rec) => Response::json(
                    &json!({
                        "ok": true,
                        "member": rec.id,
                        "display_name": rec.display_name,
                        "note": lang.t(
                            "私钥已用该成员的口令加密后存进项目；口令本身没有写进项目。",
                            "The private key is stored encrypted with that member's password; the password itself never enters the project."
                        ),
                    })
                    .to_string(),
                ),
                Err(e) => Response::error(400, &e.to_string()),
            }
        }

        "/api/access/zone" => {
            if req.method != "POST" {
                return Response::error(400, "POST only");
            }
            let body: Value = match serde_json::from_str(&req.body) {
                Ok(v) => v,
                Err(e) => return Response::error(400, &format!("body 不是 JSON：{e}")),
            };
            let path = body.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let name = body.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let label = body.get("label").and_then(|v| v.as_str()).unwrap_or("");
            let paths: Vec<String> = body
                .get("paths")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str())
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect()
                })
                .unwrap_or_default();
            let mut grants = Vec::new();
            if let Some(arr) = body.get("members").and_then(|v| v.as_array()) {
                for m in arr {
                    let id = m.get("id").and_then(|v| v.as_str()).unwrap_or("");
                    if id.is_empty() {
                        continue;
                    }
                    let perm = m
                        .get("permission")
                        .and_then(|v| v.as_str())
                        .unwrap_or("read");
                    match cercis_core::Permission::parse(perm) {
                        Ok(p) => grants.push((id.to_string(), p)),
                        Err(e) => return Response::error(400, &e.to_string()),
                    }
                }
            }
            if path.is_empty() || name.is_empty() || paths.is_empty() || grants.is_empty() {
                return Response::error(
                    400,
                    lang.t(
                        "需要 path、name、至少一条路径规则与至少一个成员",
                        "path, name, at least one path rule and one member are required",
                    ),
                );
            }
            let keep_plaintext = body
                .get("keep_plaintext")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let no_apply = body
                .get("no_apply")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            match cercis_core::access::create_zone(
                std::path::Path::new(path),
                name,
                label,
                &paths,
                &grants,
                !no_apply,
                keep_plaintext,
            ) {
                Ok((zone, report)) => Response::json(
                    &json!({
                        "ok": true,
                        "zone": zone.name,
                        "label": zone.label,
                        "generation": zone.generation,
                        "key_fingerprint": zone.key_fingerprint,
                        "paths": zone.paths,
                        "encrypted": report.as_ref().map(|r| r.encrypted.clone()).unwrap_or_default(),
                        "kept_plaintext": report.as_ref().map(|r| r.kept_plaintext.clone()).unwrap_or_default(),
                        "objects_encrypted": report.as_ref().map(|r| r.objects_encrypted).unwrap_or(0),
                    })
                    .to_string(),
                ),
                Err(e) => Response::error(400, &e.to_string()),
            }
        }

        "/api/access/grant" => {
            if req.method != "POST" {
                return Response::error(400, "POST only");
            }
            let body: Value = match serde_json::from_str(&req.body) {
                Ok(v) => v,
                Err(e) => return Response::error(400, &format!("body 不是 JSON：{e}")),
            };
            let path = body.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let zone = body.get("zone").and_then(|v| v.as_str()).unwrap_or("");
            let member = body.get("member").and_then(|v| v.as_str()).unwrap_or("");
            let perm = body
                .get("permission")
                .and_then(|v| v.as_str())
                .unwrap_or("read");
            let actor = body
                .get("as")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty());
            if path.is_empty() || zone.is_empty() || member.is_empty() {
                return Response::error(400, lang.t("需要 path、zone、member", "path, zone, member are required"));
            }
            let permission = match cercis_core::Permission::parse(perm) {
                Ok(p) => p,
                Err(e) => return Response::error(400, &e.to_string()),
            };
            match cercis_core::access::grant(
                std::path::Path::new(path),
                zone,
                member,
                permission,
                actor,
            ) {
                Ok(z) => Response::json(
                    &json!({
                        "ok": true,
                        "zone": z.name,
                        "members": z.members.iter().map(|m| json!({
                            "id": m.id, "permission": m.permission.as_str(),
                        })).collect::<Vec<_>>(),
                    })
                    .to_string(),
                ),
                Err(e) => Response::error(400, &e.to_string()),
            }
        }

        "/api/access/revoke" => {
            if req.method != "POST" {
                return Response::error(400, "POST only");
            }
            let body: Value = match serde_json::from_str(&req.body) {
                Ok(v) => v,
                Err(e) => return Response::error(400, &format!("body 不是 JSON：{e}")),
            };
            let path = body.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let zone = body.get("zone").and_then(|v| v.as_str()).unwrap_or("");
            let member = body.get("member").and_then(|v| v.as_str()).unwrap_or("");
            let actor = body
                .get("as")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty());
            if path.is_empty() || zone.is_empty() || member.is_empty() {
                return Response::error(400, lang.t("需要 path、zone、member", "path, zone, member are required"));
            }
            match cercis_core::access::revoke(
                std::path::Path::new(path),
                zone,
                member,
                actor,
            ) {
                Ok(rep) => Response::json(
                    &json!({
                        "ok": true,
                        "zone": rep.zone,
                        "removed": rep.member,
                        "generation": rep.generation,
                        "key_fingerprint": rep.key_fingerprint,
                        "reencrypted": rep.reencrypted,
                        "remaining": rep.remaining,
                    })
                    .to_string(),
                ),
                Err(e) => Response::error(400, &e.to_string()),
            }
        }

        // 插件列表（含信任状态）
        "/api/plugins" => {
            let Some(path) = req.query.get("path") else {
                return Response::error(400, lang.t("缺少 path 参数", "missing path parameter"));
            };
            match cercis_core::plugin::discover(path, crate::plugin_host::global_plugins_dir().as_deref()) {
                Ok(found) => {
                    let trust = cercis_core::TrustRecord::load(path).unwrap_or_default();
                    let pstate = cercis_core::plugin::PluginState::load(path).unwrap_or_default();
                    let list: Vec<Value> = found
                        .iter()
                        .map(|p| {
                            let trusted = trust.is_trusted(&p.manifest.id, &p.fingerprint);
                            let runnable = crate::plugin_host::is_runnable(p, trusted);
                            json!({
                                "id": p.manifest.id,
                                "name": p.manifest.name,
                                "label": if lang == Lang::Zh { &p.manifest.label_cn } else { &p.manifest.name },
                                "tier": p.manifest.tier,
                                "origin": p.origin,
                                "trusted": trusted,
                                "runnable": runnable,
                                "disabled": pstate.is_disabled(&p.manifest.id),
                                "blocked": pstate.is_blocked(&p.manifest.id),
                                "active": p.manifest.activation(path).active
                                    && !pstate.is_disabled(&p.manifest.id),
                                "inactive_reasons": p.manifest.activation(path).reasons,
                                "permissions": p.manifest.permission_summary(),
                                "can_write": p.manifest.permissions.write,
                                "entrypoint": p.manifest.entrypoint,
                                "description": p.manifest.description,
                                "ui_extensions": p.manifest.ui.iter().map(|u| json!({
                                    "kind": u.kind,
                                    "id": u.id,
                                    "label": if lang == Lang::Zh { &u.label_cn } else { &u.label_en },
                                })).collect::<Vec<_>>(),
                                // app 界面按这些声明渲染按钮（FR-P11：插件功能必须在界面有入口）
                                "commands": p.manifest.ui.iter().filter(|u| u.kind == "command").map(|u| json!({
                                    "id": u.id,
                                    "label": if lang == Lang::Zh { &u.label_cn } else { &u.label_en },
                                    "command": u.invocation(),
                                    "placement": u.placement,
                                    "enabled": runnable,
                                })).collect::<Vec<_>>(),
                            })
                        })
                        .collect();
                    Response::json(&json!({ "ok": true, "plugins": list }).to_string())
                }
                Err(e) => Response::error(500, &e.to_string()),
            }
        }

        // 运行插件命令：app 只做"发现 + 授权 + 调用"，能力本身在插件里
        "/api/plugin/run" => {
            if req.method != "POST" {
                return Response::error(400, "POST only");
            }
            let body: Value = match serde_json::from_str(&req.body) {
                Ok(v) => v,
                Err(e) => return Response::error(400, &format!("JSON 解析失败: {e}")),
            };
            let path = body.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let id = body.get("id").and_then(|v| v.as_str()).unwrap_or("");
            if path.is_empty() || id.is_empty() {
                return Response::error(400, lang.t("缺少 path 或 id", "missing path or id"));
            }
            let command = body
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let args: Vec<String> = body
                .get("args")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|i| i.as_str())
                        .map(|s| s.to_string())
                        .collect()
                })
                .unwrap_or_default();

            let found = match cercis_core::plugin::discover(path, crate::plugin_host::global_plugins_dir().as_deref()) {
                Ok(f) => f,
                Err(e) => return Response::error(500, &e.to_string()),
            };
            let Some(plugin) = crate::plugin_host::select(&found, id) else {
                return Response::error(
                    404,
                    lang.t("未找到插件（先安装或信任）", "plugin not found"),
                );
            };
            let trust = cercis_core::TrustRecord::load(path).unwrap_or_default();
            let trusted = trust.is_trusted(&plugin.manifest.id, &plugin.fingerprint);

            match crate::plugin_host::run(std::path::Path::new(path), plugin, trusted, &command, &args)
            {
                Ok(outcome) => Response::json(
                    &json!({
                        "ok": outcome.succeeded(),
                        "plugin": id,
                        "command": command,
                        "exit_code": outcome.code,
                        "command_line": outcome.command_line,
                        "write_granted": plugin.manifest.permissions.write,
                        "result": outcome.result,
                        "stdout_tail": tail(&outcome.stdout, 2000),
                        "stderr_tail": tail(&outcome.stderr, 2000),
                    })
                    .to_string(),
                ),
                // 未信任、入口缺失等：明确拒绝（FR-P04 / FR-P31）
                Err(e) => Response::error(403, &e),
            }
        }

        // 信任 / 撤销信任
        "/api/trust" => {
            if req.method != "POST" {
                return Response::error(400, "POST only");
            }
            let body: Value = match serde_json::from_str(&req.body) {
                Ok(v) => v,
                Err(e) => return Response::error(400, &format!("JSON 解析失败: {e}")),
            };
            let path = body.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let id = body.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let revoke = body
                .get("revoke")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if path.is_empty() || id.is_empty() {
                return Response::error(400, lang.t("缺少 path 或 id", "missing path or id"));
            }

            match cercis_core::plugin::discover(path, crate::plugin_host::global_plugins_dir().as_deref()) {
                Ok(found) => {
                    let target = crate::plugin_host::select(&found, id);
                    let mut tr = cercis_core::TrustRecord::load(path).unwrap_or_default();
                    if revoke {
                        tr.untrust(id);
                    } else if let Some(t) = target {
                        tr.trust(id, &t.fingerprint);
                    } else {
                        return Response::error(404, lang.t("未找到插件", "plugin not found"));
                    }
                    if let Err(e) = tr.save(path) {
                        return Response::error(500, &e.to_string());
                    }
                    Response::json(&json!({ "ok": true, "id": id, "revoked": revoke }).to_string())
                }
                Err(e) => Response::error(500, &e.to_string()),
            }
        }

        // 生成 AI 材料（AGENTS.md 等）
        "/api/agents" => {
            if req.method != "POST" {
                return Response::error(400, "POST only");
            }
            let body: Value = match serde_json::from_str(&req.body) {
                Ok(v) => v,
                Err(e) => return Response::error(400, &format!("JSON 解析失败: {e}")),
            };
            let path = body.get("path").and_then(|v| v.as_str()).unwrap_or("");
            if path.is_empty() {
                return Response::error(400, lang.t("缺少 path", "missing path"));
            }
            match cercis_core::ai::refresh_all(path) {
                Ok(_) => Response::json(&json!({ "ok": true }).to_string()),
                Err(e) => Response::error(500, &e.to_string()),
            }
        }

        // 生成机器清单
        "/api/manifest" => {
            if req.method != "POST" {
                return Response::error(400, "POST only");
            }
            let body: Value = match serde_json::from_str(&req.body) {
                Ok(v) => v,
                Err(e) => return Response::error(400, &format!("JSON 解析失败: {e}")),
            };
            let path = body.get("path").and_then(|v| v.as_str()).unwrap_or("");
            if path.is_empty() {
                return Response::error(400, lang.t("缺少 path", "missing path"));
            }
            match cercis_core::Manifest::generate(path).and_then(|m| {
                m.write(path)?;
                Ok(m)
            }) {
                Ok(m) => Response::json(
                    &json!({
                        "ok": true,
                        "tables": m.tables.len(),
                        "change_count": m.change_count,
                    })
                    .to_string(),
                ),
                Err(e) => Response::error(500, &e.to_string()),
            }
        }

        // ── 插件市场（D41–D43）────────────────────────────────────────────
        // 分工：下载在 app（这里），校验/安装/记账在内核（cercis_core::market）。
        "/api/market" => {
            let global = crate::plugin_host::global_plugins_dir();
            let user = global
                .as_ref()
                .and_then(|d| cercis_core::UserMarkets::load(d).ok())
                .unwrap_or_default();
            let markets: Vec<Value> = user
                .effective()
                .iter()
                .map(|m| {
                    // 缓存里的清单（下载过才有内容）；顺带列出"可装什么"
                    let cached = crate::market_host::cached_listing(&m.id);
                    let (cached_path, plugins) = match &cached {
                        Some((dir, listing)) => (
                            dir.display().to_string(),
                            listing
                                .plugins
                                .iter()
                                .map(|p| {
                                    json!({
                                        "id": p.id,
                                        "name": if p.label_cn.is_empty() { &p.name } else { &p.label_cn },
                                        "version": p.version,
                                        "summary": p.summary,
                                        "permissions": p.permissions,
                                        "path": p.path,
                                    })
                                })
                                .collect::<Vec<_>>(),
                        ),
                        None => (String::new(), Vec::new()),
                    };
                    json!({
                        "id": m.id,
                        "label": m.label_cn,
                        "repository": m.repository,
                        "ref": m.default_ref,
                        "bundled": m.bundled,
                        "removable": !m.bundled,
                        "cached": !cached_path.is_empty(),
                        "cache_path": cached_path,
                        "plugins": plugins,
                    })
                })
                .collect();
            let installed: Vec<Value> = crate::market_host::installed()
                .iter()
                .map(|r| {
                    json!({
                        "id": r.id,
                        "version": r.version,
                        "market": r.market,
                        "repository": r.repository,
                        "ref": r.git_ref,
                        "fingerprint": r.fingerprint,
                        "bundled": r.bundled,
                        "removable": !r.bundled,
                        "installed_at": r.installed_at,
                    })
                })
                .collect();
            // 可更新：只拿**已缓存的**清单算，不联网（刷新清单才联网）
            let installs = cercis_core::Installs::load(
                &crate::plugin_host::global_plugins_dir().unwrap_or_else(|| std::path::PathBuf::from(".")),
            )
            .unwrap_or_default();
            let updates: Vec<Value> = markets
                .iter()
                .filter_map(|m| m.get("id").and_then(|v| v.as_str()))
                .filter_map(|id| crate::market_host::cached_listing(id))
                .flat_map(|(_, listing)| cercis_core::market::check_updates(&installs, &listing))
                .filter(|u| u.actionable())
                .map(|u| {
                    json!({
                        "id": u.id,
                        "installed_version": u.installed_version,
                        "available_version": u.available_version,
                        "needs_reconfirmation": u.needs_reconfirmation,
                        "permissions_installed": u.permissions_installed,
                        "permissions_available": u.permissions_available,
                    })
                })
                .collect();
            let removed: Vec<Value> = crate::market_host::removed()
                .iter()
                .map(|r| {
                    json!({
                        "id": r.id,
                        "version": r.version,
                        "market": r.market,
                        "removed_at": r.removed_at,
                        "note": r.note,
                    })
                })
                .collect();
            Response::json(
                &json!({
                    "ok": true,
                    "global_dir": global.map(|d| d.display().to_string()),
                    "markets": markets,
                    "installed": installed,
                    "updates": updates,
                    "removed": removed,
                    "has_git": crate::market_host::has_git(),
                })
                .to_string(),
            )
        }

        // 添加市场（私有仓库也走这里；凭据由系统的 git 凭据助手处理）
        "/api/market/add" => {
            if req.method != "POST" {
                return Response::error(400, "POST only");
            }
            let body: Value = match serde_json::from_str(&req.body) {
                Ok(v) => v,
                Err(e) => return Response::error(400, &format!("JSON 解析失败: {e}")),
            };
            let repository = body
                .get("repository")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            if repository.is_empty() {
                return Response::error(400, lang.t("缺少仓库地址", "missing repository"));
            }
            let Some(global) = crate::plugin_host::global_plugins_dir() else {
                return Response::error(500, "取不到用户插件目录");
            };
            let id = body
                .get("id")
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| default_market_id(repository));
            let mut user = match cercis_core::UserMarkets::load(&global) {
                Ok(u) => u,
                Err(e) => return Response::error(500, &e.to_string()),
            };
            user.add(cercis_core::market::MarketEntry {
                id: id.clone(),
                label_cn: body
                    .get("label")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&id)
                    .to_string(),
                repository: repository.to_string(),
                marketplace_path: body
                    .get("marketplace_path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("marketplace.json")
                    .to_string(),
                default_ref: body
                    .get("ref")
                    .and_then(|v| v.as_str())
                    .unwrap_or("main")
                    .to_string(),
                bundled: false,
                added_at: cercis_core::ids::now_iso8601(),
            });
            match user.save(&global) {
                Ok(()) => Response::json(&json!({ "ok": true, "id": id }).to_string()),
                Err(e) => Response::error(500, &e.to_string()),
            }
        }

        // 移除市场（随包的基础市场会被内核拒绝）
        "/api/market/remove" => {
            if req.method != "POST" {
                return Response::error(400, "POST only");
            }
            let body: Value = match serde_json::from_str(&req.body) {
                Ok(v) => v,
                Err(e) => return Response::error(400, &format!("JSON 解析失败: {e}")),
            };
            let id = body.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let Some(global) = crate::plugin_host::global_plugins_dir() else {
                return Response::error(500, "取不到用户插件目录");
            };
            let mut user = match cercis_core::UserMarkets::load(&global) {
                Ok(u) => u,
                Err(e) => return Response::error(500, &e.to_string()),
            };
            match user.remove(id) {
                Ok(msg) => match user.save(&global) {
                    Ok(()) => Response::json(&json!({ "ok": true, "message": msg }).to_string()),
                    Err(e) => Response::error(500, &e.to_string()),
                },
                Err(e) => Response::error(400, &e.to_string()),
            }
        }

        // 下载 / 刷新市场清单（**只有这里会联网**）
        "/api/market/refresh" => {
            if req.method != "POST" {
                return Response::error(400, "POST only");
            }
            let body: Value = match serde_json::from_str(&req.body) {
                Ok(v) => v,
                Err(e) => return Response::error(400, &format!("JSON 解析失败: {e}")),
            };
            let id = body.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let Some(global) = crate::plugin_host::global_plugins_dir() else {
                return Response::error(500, "取不到用户插件目录");
            };
            let user = match cercis_core::UserMarkets::load(&global) {
                Ok(u) => u,
                Err(e) => return Response::error(500, &e.to_string()),
            };
            let Some(entry) = user.effective().into_iter().find(|m| m.id == id) else {
                return Response::error(404, &format!("没有这个市场：{id}"));
            };
            let git_ref = body
                .get("ref")
                .and_then(|v| v.as_str())
                .unwrap_or(&entry.default_ref);
            match crate::market_host::download(&entry.repository, git_ref, &entry.id) {
                Ok(dir) => match crate::market_host::load_listing(&dir) {
                    Ok(listing) => Response::json(
                        &json!({
                            "ok": true,
                            "id": entry.id,
                            "cache_path": dir.display().to_string(),
                            "name": listing.name,
                            "label": listing.label_cn,
                            "bundled": listing.bundled,
                            "plugins": listing.plugins.iter().map(|p| json!({
                                "id": p.id,
                                "name": if p.label_cn.is_empty() { &p.name } else { &p.label_cn },
                                "version": p.version,
                                "summary": p.summary,
                                "permissions": p.permissions,
                            })).collect::<Vec<_>>(),
                        })
                        .to_string(),
                    ),
                    Err(e) => Response::error(500, &e),
                },
                Err(e) => Response::error(500, &e),
            }
        }

        // 安装：缓存里没有就先下载，然后交给内核那一段
        "/api/market/install" => {
            if req.method != "POST" {
                return Response::error(400, "POST only");
            }
            let body: Value = match serde_json::from_str(&req.body) {
                Ok(v) => v,
                Err(e) => return Response::error(400, &format!("JSON 解析失败: {e}")),
            };
            let plugin = body.get("plugin").and_then(|v| v.as_str()).unwrap_or("");
            if plugin.is_empty() {
                return Response::error(400, lang.t("缺少 plugin", "missing plugin"));
            }
            // 本地目录优先（手动下载 / 拖入插件包这条路）
            let dir = match body.get("from").and_then(|v| v.as_str()) {
                Some(d) if !d.trim().is_empty() => std::path::PathBuf::from(d),
                _ => {
                    let id = body.get("id").and_then(|v| v.as_str()).unwrap_or("");
                    let Some(global) = crate::plugin_host::global_plugins_dir() else {
                        return Response::error(500, "取不到用户插件目录");
                    };
                    let user = match cercis_core::UserMarkets::load(&global) {
                        Ok(u) => u,
                        Err(e) => return Response::error(500, &e.to_string()),
                    };
                    let Some(entry) = user.effective().into_iter().find(|m| m.id == id) else {
                        return Response::error(404, &format!("没有这个市场：{id}"));
                    };
                    let git_ref = body
                        .get("ref")
                        .and_then(|v| v.as_str())
                        .unwrap_or(&entry.default_ref);
                    let cached = crate::market_host::cached_listing(&entry.id).map(|(d, _)| d);
                    match cached {
                        Some(d) => d,
                        None => match crate::market_host::download(&entry.repository, git_ref, &entry.id)
                        {
                            Ok(d) => d,
                            Err(e) => return Response::error(500, &e),
                        },
                    }
                }
            };
            let market_id = body.get("id").and_then(|v| v.as_str()).unwrap_or("local");
            let git_ref = body.get("ref").and_then(|v| v.as_str()).unwrap_or("");
            match crate::market_host::install_from(&dir, plugin, market_id, git_ref) {
                Ok((record, dest)) => Response::json(
                    &json!({
                        "ok": true,
                        "id": record.id,
                        "version": record.version,
                        "path": dest.display().to_string(),
                        "fingerprint": record.fingerprint,
                        "bundled": record.bundled,
                    })
                    .to_string(),
                ),
                Err(e) => Response::error(400, &e),
            }
        }

        // 删除（随包的基础插件会被内核拒绝，并给出停用/屏蔽的指引）
        "/api/market/uninstall" => {
            if req.method != "POST" {
                return Response::error(400, "POST only");
            }
            let body: Value = match serde_json::from_str(&req.body) {
                Ok(v) => v,
                Err(e) => return Response::error(400, &format!("JSON 解析失败: {e}")),
            };
            let id = body.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let note = body.get("note").and_then(|v| v.as_str()).unwrap_or("");
            match crate::market_host::uninstall(id, note) {
                Ok(message) => {
                    Response::json(&json!({ "ok": true, "message": message }).to_string())
                }
                Err(e) => Response::error(400, &e),
            }
        }

        // 找回被删掉的插件
        "/api/market/restore" => {
            if req.method != "POST" {
                return Response::error(400, "POST only");
            }
            let body: Value = match serde_json::from_str(&req.body) {
                Ok(v) => v,
                Err(e) => return Response::error(400, &format!("JSON 解析失败: {e}")),
            };
            let id = body.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let from = body
                .get("from")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(std::path::PathBuf::from);
            match crate::market_host::restore(id, from.as_deref()) {
                Ok(message) => {
                    Response::json(&json!({ "ok": true, "message": message }).to_string())
                }
                Err(e) => Response::error(400, &e),
            }
        }

        // 停用 / 启用 / 屏蔽 / 取消屏蔽（项目级；都写流水）
        "/api/plugin/state" => {
            if req.method != "POST" {
                return Response::error(400, "POST only");
            }
            let body: Value = match serde_json::from_str(&req.body) {
                Ok(v) => v,
                Err(e) => return Response::error(400, &format!("JSON 解析失败: {e}")),
            };
            let path = body.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let id = body.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let action = body.get("action").and_then(|v| v.as_str()).unwrap_or("");
            if path.is_empty() || id.is_empty() {
                return Response::error(400, lang.t("缺少 path 或 id", "missing path or id"));
            }
            let p = match Project::open(path, None) {
                Ok(p) => p,
                Err(e) => return Response::error(500, &e.to_string()),
            };
            let mut state = match cercis_core::plugin::PluginState::load(path) {
                Ok(s) => s,
                Err(e) => return Response::error(500, &e.to_string()),
            };
            let (verb, summary) = match action {
                "disable" => {
                    state.disable(id);
                    ("disable", format!("停用插件 {id}"))
                }
                "enable" => {
                    state.enable(id);
                    ("enable", format!("启用插件 {id}"))
                }
                "block" => {
                    state.block(id);
                    ("block", format!("屏蔽插件 {id}（不加载、界面不出现、命令不执行）"))
                }
                "unblock" => {
                    state.unblock(id);
                    ("unblock", format!("取消屏蔽插件 {id}"))
                }
                _ => {
                    return Response::error(
                        400,
                        lang.t(
                            "action 只能是 disable/enable/block/unblock",
                            "action must be disable/enable/block/unblock",
                        ),
                    )
                }
            };
            if let Err(e) = state.save(path) {
                return Response::error(500, &e.to_string());
            }
            let _ = p.store.append_change(&cercis_core::history::ChangeEntry::new(
                &p.store.identity.user_id,
                &p.store.identity.device_for_log(),
                verb,
                "plugin",
                id,
                &summary,
                "",
                "",
            ));
            Response::json(&json!({ "ok": true, "action": verb, "id": id }).to_string())
        }

        // 期望状态：看差异（plugins/desired.yaml）
        "/api/plugins/desired" => {
            let path = req.query.get("path").map(|s| s.as_str()).unwrap_or("");
            if path.is_empty() {
                return Response::error(400, lang.t("缺少 path", "missing path"));
            }
            let Some(desired) = (match cercis_core::Desired::load_optional(path) {
                Ok(d) => d,
                Err(e) => return Response::error(500, &e.to_string()),
            }) else {
                return Response::json(
                    &json!({
                        "ok": true,
                        "present": false,
                        "hint": lang.t(
                            "还没有 plugins/desired.yaml：写下「想要装哪些插件」，就能一键对齐",
                            "No plugins/desired.yaml yet",
                        ),
                    })
                    .to_string(),
                );
            };
            let d = desired_diff(path, &desired);
            Response::json(
                &json!({
                    "ok": true,
                    "present": true,
                    "aligned": d.is_empty(),
                    "diff": d,
                })
                .to_string(),
            )
        }

        // 期望状态：写入（app 里编辑后保存）
        "/api/plugins/desired/write" => {
            if req.method != "POST" {
                return Response::error(400, "POST only");
            }
            let body: Value = match serde_json::from_str(&req.body) {
                Ok(v) => v,
                Err(e) => return Response::error(400, &format!("JSON 解析失败: {e}")),
            };
            let path = body.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let Some(text) = body.get("yaml").and_then(|v| v.as_str()) else {
                return Response::error(400, lang.t("缺少 yaml", "missing yaml"));
            };
            let desired = match cercis_core::Desired::from_yaml(text) {
                Ok(d) => d,
                Err(e) => return Response::error(400, &e.to_string()),
            };
            match desired.save(path) {
                Ok(()) => Response::json(&json!({ "ok": true }).to_string()),
                Err(e) => Response::error(500, &e.to_string()),
            }
        }

        // 期望状态：对齐（改状态 + 用**已缓存**的市场目录把缺的装上；不联网）
        "/api/plugins/reconcile" => {
            if req.method != "POST" {
                return Response::error(400, "POST only");
            }
            let body: Value = match serde_json::from_str(&req.body) {
                Ok(v) => v,
                Err(e) => return Response::error(400, &format!("JSON 解析失败: {e}")),
            };
            let path = body.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let dry_run = body.get("dry_run").and_then(|v| v.as_bool()).unwrap_or(false);
            let Some(desired) = (match cercis_core::Desired::load_optional(path) {
                Ok(d) => d,
                Err(e) => return Response::error(500, &e.to_string()),
            }) else {
                return Response::error(404, "还没有 plugins/desired.yaml");
            };
            let d = desired_diff(path, &desired);

            let mut applied: Vec<String> = Vec::new();
            for id in &d.to_disable {
                applied.push(format!("停用 {id}"));
            }
            for id in &d.to_block {
                applied.push(format!("屏蔽 {id}"));
            }
            for id in &d.to_unblock {
                applied.push(format!("取消屏蔽 {id}"));
            }
            for id in &d.to_enable {
                applied.push(format!("启用 {id}"));
            }

            let mut installed: Vec<Value> = Vec::new();
            let mut skipped: Vec<String> = Vec::new();
            let mut state = match cercis_core::plugin::PluginState::load(path) {
                Ok(s) => s,
                Err(e) => return Response::error(500, &e.to_string()),
            };
            if !dry_run && !applied.is_empty() {
                for id in &d.to_disable {
                    state.disable(id);
                }
                for id in &d.to_block {
                    state.block(id);
                }
                for id in &d.to_unblock {
                    state.unblock(id);
                }
                for id in &d.to_enable {
                    state.enable(id);
                }
                if let Err(e) = state.save(path) {
                    return Response::error(500, &e.to_string());
                }
                if let Ok(p) = Project::open(path, None) {
                    let _ = p.store.append_change(&cercis_core::history::ChangeEntry::new(
                        &p.store.identity.user_id,
                        &p.store.identity.device_for_log(),
                        "reconcile",
                        "plugin",
                        "",
                        &format!("按期望状态对齐：{}", applied.join("、")),
                        "",
                        "",
                    ));
                }
            }

            // 缺的插件：优先用缓存里的市场目录装（app 已经下载过的那种）
            for w in &d.missing {
                let Some((dir, _listing)) = crate::market_host::cached_listing(&w.market) else {
                    skipped.push(format!(
                        "{}：市场 {} 的清单还没下载——先在上面点「下载/刷新清单」",
                        w.id, w.market
                    ));
                    continue;
                };
                if dry_run {
                    installed.push(json!({ "id": w.id, "planned": true }));
                    continue;
                }
                match crate::market_host::install_from(&dir, &w.id, &w.market, &w.git_ref) {
                    Ok((record, _dest)) => {
                        installed.push(json!({ "id": record.id, "version": record.version }))
                    }
                    Err(e) => skipped.push(format!("{}：{e}", w.id)),
                }
            }

            Response::json(
                &json!({
                    "ok": true,
                    "dry_run": dry_run,
                    "diff": d,
                    "applied": applied,
                    "installed": installed,
                    "skipped": skipped,
                })
                .to_string(),
            )
        }

        _ => Response::error(404, "unknown endpoint"),
    }
}

/// 期望状态对账（发现 + 台账 + 状态 → 差异）。
fn desired_diff(project: &str, desired: &cercis_core::Desired) -> cercis_core::DesiredDiff {
    let found = cercis_core::plugin::discover(project, crate::plugin_host::global_plugins_dir().as_deref())
        .unwrap_or_default();
    let known: Vec<String> = found.iter().map(|p| p.manifest.id.clone()).collect();
    let installs = crate::plugin_host::global_plugins_dir()
        .and_then(|d| cercis_core::Installs::load(&d).ok())
        .unwrap_or_default();
    let state = cercis_core::plugin::PluginState::load(project).unwrap_or_default();
    cercis_core::desired::diff(desired, &installs, &state, &known)
}

/// 待办标准列（§5.2.11）。
const TASK_COLUMNS: [&str; 9] = [
    "task_id",
    "title",
    "due_date",
    "status",
    "priority",
    "related_event_id",
    "notes",
    "created_at",
    "done_at",
];

/// 读待办：返回（表是否存在，按"今天/本周/逾期"分好组的任务）。
///
/// 分组规则（界面侧语义，内核不管）：
/// - `overdue`：到期日 < 今天，且未完成
/// - `today`：到期日 == 今天，且未完成
/// - `this_week`：今天 < 到期日 ≤ 本周日，且未完成
/// - `later`：更晚，或没写到期日
/// - `done`：已完成（不论日期）
fn read_tasks(path: &str) -> Result<(bool, Value), cercis_core::CercisError> {
    let p = Project::open(path, None)?;
    let table = match p.store.load_table("tasks", &[]) {
        Ok(t) => t,
        Err(_) => return Ok((false, json!({}))),
    };
    let today = chrono::Local::now().date_naive();
    // 本周日（周一为一周之始）
    let weekday = today.weekday().num_days_from_monday() as i64;
    let week_end = today + chrono::Duration::days(6 - weekday);

    let mut buckets: std::collections::BTreeMap<String, Vec<Value>> =
        std::collections::BTreeMap::new();
    for key in ["overdue", "today", "this_week", "later", "done"] {
        buckets.insert(key.to_string(), Vec::new());
    }
    for r in &table.rows {
        let status = r.get("status").cloned().unwrap_or_default();
        let done = status.contains("完成") || status.eq_ignore_ascii_case("done");
        let due_raw = r.get("due_date").cloned().unwrap_or_default();
        let due = chrono::NaiveDate::parse_from_str(due_raw.trim(), "%Y-%m-%d").ok();
        let bucket = if done {
            "done"
        } else {
            match due {
                Some(d) if d < today => "overdue",
                Some(d) if d == today => "today",
                Some(d) if d <= week_end => "this_week",
                _ => "later",
            }
        };
        buckets.get_mut(bucket).unwrap().push(json!({
            "task_id": r.get("task_id"),
            "title": r.get("title"),
            "due_date": due_raw,
            "status": status,
            "priority": r.get("priority"),
            "related_event_id": r.get("related_event_id"),
            "notes": r.get("notes"),
            "created_at": r.get("created_at"),
            "done_at": r.get("done_at"),
            "bucket": bucket,
        }));
    }
    let mut out = serde_json::Map::new();
    for (k, v) in buckets {
        out.insert(k, Value::Array(v));
    }
    Ok((true, Value::Object(out)))
}

/// 建一条待办（没有表就按标准列建）。
fn add_task(path: &str, body: &Value) -> Result<String, cercis_core::CercisError> {
    let p = Project::open(path, None)?;
    let _ = p.store.load_table("tasks", &TASK_COLUMNS)?;
    let today = chrono::Local::now().date_naive();
    let seq = p
        .store
        .load_table("tasks", &[])
        .map(|t| t.rows.len() + 1)
        .unwrap_or(1);
    let id = format!("TK-{}-{:03}", today.format("%Y%m%d"), seq);
    let mut row = cercis_core::table::Row::new();
    row.insert("task_id".into(), id.clone());
    row.insert(
        "title".into(),
        body.get("title").and_then(|v| v.as_str()).unwrap_or("").to_string(),
    );
    row.insert(
        "due_date".into(),
        body.get("due_date").and_then(|v| v.as_str()).unwrap_or("").to_string(),
    );
    row.insert("status".into(), "待办".into());
    row.insert(
        "priority".into(),
        body.get("priority").and_then(|v| v.as_str()).unwrap_or("中").to_string(),
    );
    row.insert(
        "related_event_id".into(),
        body.get("related_event_id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
    );
    row.insert(
        "notes".into(),
        body.get("notes").and_then(|v| v.as_str()).unwrap_or("").to_string(),
    );
    row.insert("created_at".into(), today.format("%Y-%m-%d").to_string());
    row.insert("done_at".into(), String::new());
    p.store
        .append_row("tasks", &TASK_COLUMNS, row, &id, "新增待办")?;
    Ok(id)
}

/// 勾选完成/取消：**改当前行 + 写一条更正记录**。
///
/// 待办状态是"可变字段"：改它时把前值交给历史对象库保管（任何一次改动都能还原），
/// 同时在 `corrections.csv` 留一条"旧值 → 新值 + 原因"，与其它更正同一本账。
fn set_task_done(
    path: &str,
    task_id: &str,
    done: bool,
) -> Result<(String, String), cercis_core::CercisError> {
    let p = Project::open(path, None)?;
    let mut table = p.store.load_table("tasks", &TASK_COLUMNS)?;
    let today = chrono::Local::now().date_naive().format("%Y-%m-%d").to_string();
    let new_status = if done { "已完成" } else { "待办" };
    let new_done_at = if done { today.clone() } else { String::new() };
    let mut old_status = String::new();
    let mut found = false;
    for r in table.rows.iter_mut() {
        if r.get("task_id").map(|s| s.as_str()) == Some(task_id) {
            old_status = r.get("status").cloned().unwrap_or_default();
            r.insert("status".into(), new_status.to_string());
            r.insert("done_at".into(), new_done_at.clone());
            found = true;
            break;
        }
    }
    if !found {
        return Err(cercis_core::CercisError::NotFound(format!(
            "没有这条待办：{task_id}"
        )));
    }
    p.store.save_table(
        &table,
        "update",
        &format!("勾选待办 {task_id}：{old_status} → {new_status}"),
    )?;
    p.store.correct(
        "tasks",
        task_id,
        "status",
        &old_status,
        new_status,
        if done { "在 app 里勾选完成" } else { "在 app 里取消完成" },
        "app:todos",
    )?;
    Ok((old_status, new_status.to_string()))
}

/// 从仓库地址推一个市场 id（`.../cercis-kits-research.git` → `cercis-kits-research`）。
fn default_market_id(repository: &str) -> String {
    repository
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("market")
        .trim_end_matches(".git")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(method: &str, path: &str, query: &[(&str, &str)], body: &str) -> Request {
        Request {
            method: method.to_string(),
            path: path.to_string(),
            query: query
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            body: body.to_string(),
        }
    }

    /// 加密共享的运行环境：**进程内只设一次**（测试是并行跑的，
    /// 谁都不许在跑的过程中把环境变量删掉，否则另一个测试会跑去写真实 HOME）。
    fn share_env() -> std::path::PathBuf {
        static HOME: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
        HOME.get_or_init(|| {
            let home = std::env::temp_dir().join("cercis-app-share-home");
            let _ = std::fs::remove_dir_all(&home);
            let _ = std::fs::create_dir_all(&home);
            std::env::set_var("CERCIS_SHARE_HOME", &home);
            std::env::set_var("CERCIS_SHARE_KDF", "light");
            std::env::set_var("CERCIS_KEYCHAIN", "off");
            home
        })
        .clone()
    }

    /// S11：app 侧的加密区域端点——状态可读、打开项目时会"锁着"、解锁后可用、上锁能收尾。
    #[test]
    fn access_endpoints_report_and_unlock() {
        let dir = std::env::temp_dir().join("cercis-app-access-test");
        let _ = std::fs::remove_dir_all(&dir);
        let project = dir.join("proj");
        let _share_home = share_env();

        let p = cercis_core::Project::create(&project, "app 加密试验", Default::default()).unwrap();
        let mut row = cercis_core::table::Row::new();
        row.insert("record_id".into(), "R-1".into());
        row.insert("secret_note".into(), "机密X9".into());
        p.store
            .append_row_extending("records", &[], row, "R-1", "写一条用于加密的测试数据")
            .unwrap();
        cercis_core::access::create_member(
            p.root(),
            "alice",
            "张老师",
            "alice-pw",
            cercis_core::crypto::KdfParams::light(),
            "",
        )
        .unwrap();
        cercis_core::access::create_zone(
            p.root(),
            "secret-records",
            "机密的",
            &["tables/records.csv".to_string()],
            &[("alice".to_string(), cercis_core::Permission::Admin)],
            true,
            false,
        )
        .unwrap();

        let path = project.to_str().unwrap();
        let res = handle(&req("GET", "/api/access", &[("path", path)], ""));
        let body: Value = serde_json::from_str(&res.body).unwrap();
        assert_eq!(body["ok"], true);
        assert_eq!(body["zones"][0]["zone"], "secret-records");
        assert_eq!(body["zones"][0]["files"], 1);
        assert_eq!(body["zones"][0]["permission"], Value::Null);

        let res = handle(&req("GET", "/api/open", &[("path", path)], ""));
        let body: Value = serde_json::from_str(&res.body).unwrap();
        let records = body["tables"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == "records")
            .expect("模板里的表要出现在概览里");
        assert_eq!(records["locked"], true);

        let res = handle(&req(
            "POST",
            "/api/access/unlock",
            &[],
            &format!(r#"{{"path":"{path}","member":"alice","password":"alice-pw"}}"#),
        ));
        let body: Value = serde_json::from_str(&res.body).unwrap();
        assert_eq!(body["ok"], true, "{body}");
        assert_eq!(body["zones"][0]["permission"], "admin");

        let res = handle(&req("GET", "/api/open", &[("path", path)], ""));
        let body: Value = serde_json::from_str(&res.body).unwrap();
        let records = body["tables"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == "records")
            .unwrap();
        assert_eq!(records["locked"], false);
        assert_eq!(records["rows"], 1);

        let res = handle(&req(
            "POST",
            "/api/access/lock",
            &[],
            &format!(r#"{{"path":"{path}"}}"#),
        ));
        let body: Value = serde_json::from_str(&res.body).unwrap();
        assert_eq!(body["ok"], true);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// S13：app 里的"搬运"——打包 → 合并 → 看报告，全走内核。
    #[test]
    fn transfer_endpoints_pack_and_merge() {
        let dir = std::env::temp_dir().join("cercis-app-transfer-test");
        let _ = std::fs::remove_dir_all(&dir);
        let a = dir.join("a");
        let b = dir.join("b");
        let p = cercis_core::Project::create(&a, "搬运试验", Default::default()).unwrap();
        let mut row = cercis_core::table::Row::new();
        row.insert("record_id".into(), "R-1".into());
        row.insert("value_num".into(), "1".into());
        p.store
            .append_row_extending("records", &[], row, "R-1", "app 打包测试")
            .unwrap();
        // 第二台设备（复制一份），各加一条
        copy_dir(&a, &b);
        {
            let pb = cercis_core::Project::open(&b, None).unwrap();
            let mut row = cercis_core::table::Row::new();
            row.insert("record_id".into(), "R-2".into());
            row.insert("value_num".into(), "2".into());
            pb.store
                .append_row_extending("records", &[], row, "R-2", "第二台设备")
                .unwrap();
        }

        // 打包（b → 包）
        let pack_dir = dir.join("包");
        let res = handle(&req(
            "POST",
            "/api/pack",
            &[],
            &format!(
                r#"{{"path":"{}","out":"{}","part_size_mb":20}}"#,
                b.display(),
                pack_dir.display()
            ),
        ));
        let body: Value = serde_json::from_str(&res.body).unwrap();
        assert_eq!(body["ok"], true, "{body}");
        assert!(pack_dir.join("pack.json").exists());

        // 合并进 a
        let res = handle(&req(
            "POST",
            "/api/merge",
            &[],
            &format!(
                r#"{{"pack":"{}","into":"{}"}}"#,
                pack_dir.display(),
                a.display()
            ),
        ));
        let body: Value = serde_json::from_str(&res.body).unwrap();
        assert_eq!(body["ok"], true, "{body}");
        assert!(body["summary"].as_str().unwrap().contains("并集"));
        let report = body["report_path"].as_str().unwrap().to_string();
        assert!(std::path::Path::new(&report).exists());

        // 报告列表 + 读报告
        let res = handle(&req("GET", "/api/reports", &[("path", a.to_str().unwrap())], ""));
        let body: Value = serde_json::from_str(&res.body).unwrap();
        let name = body["reports"][0]["name"].as_str().unwrap().to_string();
        assert!(name.starts_with("merge-report-"));
        let res = handle(&req(
            "GET",
            "/api/report",
            &[("path", a.to_str().unwrap()), ("file", &name)],
            "",
        ));
        let body: Value = serde_json::from_str(&res.body).unwrap();
        assert_eq!(body["ok"], true);
        assert!(body["text"].as_str().unwrap().contains("合并报告"));

        // 目录穿越会被挡
        let res = handle(&req(
            "GET",
            "/api/report",
            &[("path", a.to_str().unwrap()), ("file", "../project.yaml")],
            "",
        ));
        let body: Value = serde_json::from_str(&res.body).unwrap();
        assert_eq!(body["ok"], false);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// S13：app 里的加密区域管理——建成员、建区域（立刻加密）、授权、移除。
    #[test]
    fn access_admin_endpoints_manage_members_and_zones() {
        let dir = std::env::temp_dir().join("cercis-app-zone-admin-test");
        let _ = std::fs::remove_dir_all(&dir);
        let project = dir.join("proj");
        let _share_home = share_env();
        let p = cercis_core::Project::create(&project, "区域管理", Default::default()).unwrap();
        let mut row = cercis_core::table::Row::new();
        row.insert("record_id".into(), "R-1".into());
        row.insert("secret_note".into(), "机密Z7".into());
        p.store
            .append_row_extending("records", &[], row, "R-1", "写一条")
            .unwrap();
        let path = project.to_str().unwrap();

        // 建两个成员
        for (id, pw) in [("alice", "alice-pw"), ("bob", "bob-pw")] {
            let res = handle(&req(
                "POST",
                "/api/access/member",
                &[],
                &format!(
                    r#"{{"path":"{path}","id":"{id}","name":"{id}","password":"{pw}"}}"#
                ),
            ));
            let body: Value = serde_json::from_str(&res.body).unwrap();
            assert_eq!(body["ok"], true, "{body}");
        }

        // 建区域（立刻加密）：alice 管理、bob 只读
        let res = handle(&req(
            "POST",
            "/api/access/zone",
            &[],
            &format!(
                r#"{{"path":"{path}","name":"secret","label":"机密的","paths":["tables/records.csv"],
                    "members":[{{"id":"alice","permission":"admin"}},{{"id":"bob","permission":"read"}}]}}"#
            ),
        ));
        let body: Value = serde_json::from_str(&res.body).unwrap();
        assert_eq!(body["ok"], true, "{body}");
        assert_eq!(body["generation"], 1);
        assert_eq!(body["encrypted"].as_array().unwrap().len(), 1);
        assert!(!project.join("tables/records.csv").exists(), "明文应已换成密文");

        // 授权：把 bob 提到可写（alice 已在上面解锁过？没有——先解锁）
        let res = handle(&req(
            "POST",
            "/api/access/unlock",
            &[],
            &format!(r#"{{"path":"{path}","member":"alice","password":"alice-pw"}}"#),
        ));
        let body: Value = serde_json::from_str(&res.body).unwrap();
        assert_eq!(body["ok"], true, "{body}");

        let res = handle(&req(
            "POST",
            "/api/access/grant",
            &[],
            &format!(
                r#"{{"path":"{path}","zone":"secret","member":"bob","permission":"write","as":"alice"}}"#
            ),
        ));
        let body: Value = serde_json::from_str(&res.body).unwrap();
        assert_eq!(body["ok"], true, "{body}");
        assert!(body["members"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["id"] == "bob" && m["permission"] == "write"));

        // 移除 bob：换密钥 + 重加密
        let res = handle(&req(
            "POST",
            "/api/access/revoke",
            &[],
            &format!(r#"{{"path":"{path}","zone":"secret","member":"bob","as":"alice"}}"#),
        ));
        let body: Value = serde_json::from_str(&res.body).unwrap();
        assert_eq!(body["ok"], true, "{body}");
        assert_eq!(body["generation"], 2);
        assert_eq!(body["reencrypted"], 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FR-N08/N09：app 里的账户——建、登录、看、改角色、删；打开项目时告诉界面要不要登录。
    #[test]
    fn account_endpoints_login_and_gate_writes() {
        let dir = std::env::temp_dir().join("cercis-app-account-test");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = share_env();
        let project = dir.join("proj");
        let p = cercis_core::Project::create(&project, "多人试验", Default::default()).unwrap();
        let path = project.to_str().unwrap();

        // 开放模式：写得了，打开项目时 account_required=false
        let mut row = cercis_core::table::Row::new();
        row.insert("record_id".into(), "R-1".into());
        p.store
            .append_row_extending("records", &[], row.clone(), "R-1", "开放模式写入")
            .unwrap();
        let res = handle(&req("GET", "/api/open", &[("path", path)], ""));
        let body: Value = serde_json::from_str(&res.body).unwrap();
        assert_eq!(body["account_required"], false);
        assert!(body["whoami"].is_null());

        // 建第一个账户（必须是管理员）
        let res = handle(&req(
            "POST",
            "/api/account/add",
            &[],
            &format!(
                r#"{{"path":"{path}","id":"alice","name":"张老师","role":"admin","password":"pw-a"}}"#
            ),
        ));
        let body: Value = serde_json::from_str(&res.body).unwrap();
        assert_eq!(body["ok"], true, "{body}");

        // 现在：打开项目会说"要登录"，没登录写不了
        let res = handle(&req("GET", "/api/open", &[("path", path)], ""));
        let body: Value = serde_json::from_str(&res.body).unwrap();
        assert_eq!(body["account_required"], true);
        assert!(body["whoami"].is_null(), "还没登录：{body}");
        let err = p
            .store
            .append_row_extending("records", &[], row.clone(), "R-2", "没登录")
            .unwrap_err();
        assert!(err.to_string().contains("登录"), "{err}");

        // 口令错了：登录失败且不建立登录态
        let res = handle(&req(
            "POST",
            "/api/account/login",
            &[],
            &format!(r#"{{"path":"{path}","id":"alice","password":"wrong"}}"#),
        ));
        let body: Value = serde_json::from_str(&res.body).unwrap();
        assert_eq!(body["ok"], false);
        assert!(cercis_core::accounts::current(&project).unwrap().is_none());

        // 正确口令：能登录，写入放行
        let res = handle(&req(
            "POST",
            "/api/account/login",
            &[],
            &format!(r#"{{"path":"{path}","id":"alice","password":"pw-a"}}"#),
        ));
        let body: Value = serde_json::from_str(&res.body).unwrap();
        assert_eq!(body["ok"], true, "{body}");
        assert_eq!(body["role"], "admin");
        p.store
            .append_row_extending("records", &[], row, "R-2", "登录后写入")
            .unwrap();

        // 账户列表能看，且没有明文口令
        let res = handle(&req("GET", "/api/account", &[("path", path)], ""));
        let body: Value = serde_json::from_str(&res.body).unwrap();
        assert_eq!(body["enabled"], true);
        assert_eq!(body["accounts"][0]["id"], "alice");
        assert_eq!(body["whoami"]["id"], "alice");
        let text = std::fs::read_to_string(project.join("access/accounts.yaml")).unwrap();
        assert!(!text.contains("pw-a"), "不能存明文口令");

        // 退出登录：又写不了了
        let res = handle(&req(
            "POST",
            "/api/account/logout",
            &[],
            &format!(r#"{{"path":"{path}"}}"#),
        ));
        let body: Value = serde_json::from_str(&res.body).unwrap();
        assert_eq!(body["ok"], true);
        assert!(cercis_core::accounts::current(&project).unwrap().is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 待办（§5.2.11）：加一条 → 分桶（逾期/今天/本周）→ 勾选完成 → 留更正与流水。
    #[test]
    fn todo_endpoints_add_bucket_and_complete() {
        let dir = std::env::temp_dir().join("cercis-app-todos-test");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = share_env();
        let project = dir.join("proj");
        cercis_core::Project::create(&project, "待办试验", Default::default()).unwrap();
        let path = project.to_str().unwrap();

        // 还没有 tasks.csv
        let res = handle(&req("GET", "/api/todos", &[("path", path)], ""));
        let body: Value = serde_json::from_str(&res.body).unwrap();
        assert_eq!(body["ok"], true);
        assert_eq!(body["exists"], false);

        // 加三条：昨天（逾期）、今天、下个月
        let today = chrono::Local::now().date_naive();
        let yesterday = today - chrono::Duration::days(1);
        let later = today + chrono::Duration::days(40);
        for (title, due) in [
            ("昨天的活", yesterday.format("%Y-%m-%d").to_string()),
            ("今天的活", today.format("%Y-%m-%d").to_string()),
            ("下个月的活", later.format("%Y-%m-%d").to_string()),
        ] {
            let res = handle(&req(
                "POST",
                "/api/todos/add",
                &[],
                &format!(r#"{{"path":"{path}","title":"{title}","due_date":"{due}","priority":"高"}}"#),
            ));
            let body: Value = serde_json::from_str(&res.body).unwrap();
            assert_eq!(body["ok"], true, "{body}");
        }

        let res = handle(&req("GET", "/api/todos", &[("path", path)], ""));
        let body: Value = serde_json::from_str(&res.body).unwrap();
        assert_eq!(body["exists"], true);
        assert_eq!(body["tasks"]["overdue"].as_array().unwrap().len(), 1);
        assert_eq!(body["tasks"]["today"].as_array().unwrap().len(), 1);
        assert_eq!(body["tasks"]["later"].as_array().unwrap().len(), 1);
        let overdue_id = body["tasks"]["overdue"][0]["task_id"].as_str().unwrap().to_string();

        // 勾选完成：改当前行 + 写更正记录
        let res = handle(&req(
            "POST",
            "/api/todos/done",
            &[],
            &format!(
                r#"{{"path":"{path}","task_id":"{overdue_id}","done":true}}"#
            ),
        ));
        let body: Value = serde_json::from_str(&res.body).unwrap();
        assert_eq!(body["ok"], true, "{body}");
        assert_eq!(body["old"], "待办");
        assert_eq!(body["new"], "已完成");

        let res = handle(&req("GET", "/api/todos", &[("path", path)], ""));
        let body: Value = serde_json::from_str(&res.body).unwrap();
        assert_eq!(body["tasks"]["overdue"].as_array().unwrap().len(), 0);
        assert_eq!(body["tasks"]["done"].as_array().unwrap().len(), 1);

        // 更正表里有记录，流水里也有
        let corrections =
            std::fs::read_to_string(project.join("tables/corrections.csv")).unwrap();
        assert!(corrections.contains(&overdue_id), "{corrections}");
        assert!(corrections.contains("已完成"), "{corrections}");
        let log = std::fs::read_to_string(project.join("tables/change_log.csv")).unwrap();
        assert!(log.contains("勾选待办"), "{log}");

        // 时间线：这条记录的版本链看得到更正
        let res = handle(&req(
            "GET",
            "/api/history/record",
            &[("path", path), ("table", "tasks"), ("id", &overdue_id)],
            "",
        ));
        let body: Value = serde_json::from_str(&res.body).unwrap();
        assert_eq!(body["ok"], true);
        assert_eq!(body["corrections"].as_array().unwrap().len(), 1);
        assert!(body["entries"].as_array().unwrap().len() >= 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 历史审阅：按表/人筛流水（FR-M07）。
    #[test]
    fn history_filters_by_table_and_user() {
        let dir = std::env::temp_dir().join("cercis-app-history-test");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = share_env();
        let project = dir.join("proj");
        let p = cercis_core::Project::create(&project, "历史试验", Default::default()).unwrap();
        let mut row = cercis_core::table::Row::new();
        row.insert("record_id".into(), "R-1".into());
        p.store
            .append_row_extending("records", &[], row, "R-1", "写一条")
            .unwrap();
        let path = project.to_str().unwrap();

        let res = handle(&req("GET", "/api/history", &[("path", path)], ""));
        let all: Value = serde_json::from_str(&res.body).unwrap();
        assert!(all["count"].as_u64().unwrap() >= 2, "{all}");

        let res = handle(&req(
            "GET",
            "/api/history",
            &[("path", path), ("table", "records")],
            "",
        ));
        let only_records: Value = serde_json::from_str(&res.body).unwrap();
        assert!(only_records["entries"]
            .as_array()
            .unwrap()
            .iter()
            .all(|e| e["target_table"] == "records"));

        let res = handle(&req(
            "GET",
            "/api/history",
            &[("path", path), ("user", "查无此人")],
            "",
        ));
        let none: Value = serde_json::from_str(&res.body).unwrap();
        assert_eq!(none["count"], 0);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// NAS：登记网络位置（本机目录当 NAS）→ 看到它 → 体检 → 移除；工作副本建/同步。
    #[test]
    fn nas_and_workcopy_endpoints() {
        let dir = std::env::temp_dir().join("cercis-app-nas-test");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = share_env();
        let main = dir.join("主副本");
        let copy = dir.join("本地副本");
        let p = cercis_core::Project::create(&main, "NAS 试验", Default::default()).unwrap();
        let mut row = cercis_core::table::Row::new();
        row.insert("record_id".into(), "R-1".into());
        p.store
            .append_row_extending("records", &[], row, "R-1", "主副本写入")
            .unwrap();
        let main_s = main.to_str().unwrap();

        // 登记
        let res = handle(&req(
            "POST",
            "/api/remote/add",
            &[],
            &format!(
                r#"{{"path":"{main_s}","name":"nas","remote_path":"{main_s}","kind":"local"}}"#
            ),
        ));
        let body: Value = serde_json::from_str(&res.body).unwrap();
        assert_eq!(body["ok"], true, "{body}");

        // 列表里能看到，且认得出"可达"
        let res = handle(&req("GET", "/api/remote", &[("path", main_s)], ""));
        let body: Value = serde_json::from_str(&res.body).unwrap();
        assert_eq!(body["remotes"][0]["name"], "nas");
        assert_eq!(body["remotes"][0]["reachable"], true);

        // 体检
        let res = handle(&req(
            "GET",
            "/api/remote/test",
            &[("path", main_s), ("name", "nas")],
            "",
        ));
        let body: Value = serde_json::from_str(&res.body).unwrap();
        assert_eq!(body["ok"], true, "{body}");
        assert_eq!(body["test"]["writable"], true);

        // 写租约：取了之后列表里有
        let res = handle(&req(
            "POST",
            "/api/remote/claim",
            &[],
            &format!(r#"{{"path":"{main_s}","who":"wenhuan@laptop","minutes":30}}"#),
        ));
        let body: Value = serde_json::from_str(&res.body).unwrap();
        assert_eq!(body["ok"], true, "{body}");
        let res = handle(&req("GET", "/api/remote", &[("path", main_s)], ""));
        let body: Value = serde_json::from_str(&res.body).unwrap();
        assert_eq!(body["leases"].as_array().unwrap().len(), 1);
        let res = handle(&req(
            "POST",
            "/api/remote/release",
            &[],
            &format!(r#"{{"path":"{main_s}","who":"wenhuan@laptop"}}"#),
        ));
        let body: Value = serde_json::from_str(&res.body).unwrap();
        assert!(body["cleared"].as_u64().unwrap() >= 1);

        // 工作副本：建 → 两边各自记 → 同步 → 两边都齐
        let res = handle(&req(
            "POST",
            "/api/workcopy/create",
            &[],
            &format!(
                r#"{{"main":"{main_s}","to":"{}","name":"nas"}}"#,
                copy.display()
            ),
        ));
        let body: Value = serde_json::from_str(&res.body).unwrap();
        assert_eq!(body["ok"], true, "{body}");

        let pc = cercis_core::Project::open(&copy, None).unwrap();
        let mut row = cercis_core::table::Row::new();
        row.insert("record_id".into(), "R-2".into());
        pc.store
            .append_row_extending("records", &[], row, "R-2", "副本写入")
            .unwrap();
        let mut row = cercis_core::table::Row::new();
        row.insert("record_id".into(), "R-3".into());
        p.store
            .append_row_extending("records", &[], row, "R-3", "主副本再写一条")
            .unwrap();

        let copy_s = copy.to_str().unwrap();
        let res = handle(&req("GET", "/api/workcopy", &[("path", copy_s)], ""));
        let body: Value = serde_json::from_str(&res.body).unwrap();
        assert_eq!(body["is_workcopy"], true);
        assert_eq!(body["status"]["verdict"], "needs-sync", "{body}");

        let res = handle(&req(
            "POST",
            "/api/workcopy/sync",
            &[],
            &format!(r#"{{"copy":"{copy_s}","dry_run":false}}"#),
        ));
        let body: Value = serde_json::from_str(&res.body).unwrap();
        assert_eq!(body["ok"], true, "{body}");
        assert_eq!(body["conflicts_total"], 0);

        for root in [&main, &copy] {
            let t = cercis_core::table::Table::load(root.join("tables/records.csv")).unwrap();
            assert_eq!(t.rows.len(), 3, "{root:?} 应有 3 条");
        }

        // 移除登记
        let res = handle(&req(
            "POST",
            "/api/remote/remove",
            &[],
            &format!(r#"{{"path":"{main_s}","name":"nas"}}"#),
        ));
        let body: Value = serde_json::from_str(&res.body).unwrap();
        assert_eq!(body["ok"], true);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 大表：分页（offset/limit/total）与排序（点表头走的就是这个接口参数）。
    #[test]
    fn table_endpoint_paginates_and_sorts() {
        let dir = std::env::temp_dir().join("cercis-app-page-test");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = share_env();
        let project = dir.join("proj");
        let p = cercis_core::Project::create(&project, "大表试验", Default::default()).unwrap();
        let mut rows = Vec::new();
        for i in 0..250 {
            let mut row = cercis_core::table::Row::new();
            row.insert("record_id".into(), format!("R-{i:04}"));
            row.insert("value_num".into(), i.to_string());
            rows.push(row);
        }
        p.store
            .append_rows("records", &[], rows, "import", "批量造 250 行", "test")
            .unwrap();
        let path = project.to_str().unwrap();

        // 第一页
        let res = handle(&req(
            "GET",
            "/api/table",
            &[("path", path), ("table", "records"), ("limit", "100"), ("offset", "0")],
            "",
        ));
        let body: Value = serde_json::from_str(&res.body).unwrap();
        assert_eq!(body["count"], 100);
        assert_eq!(body["total"], 250);
        assert_eq!(body["offset"], 0);
        assert_eq!(body["rows"][0]["record_id"], "R-0000");

        // 第三页
        let res = handle(&req(
            "GET",
            "/api/table",
            &[("path", path), ("table", "records"), ("limit", "100"), ("offset", "200")],
            "",
        ));
        let body: Value = serde_json::from_str(&res.body).unwrap();
        assert_eq!(body["count"], 50, "最后一页只有 50 行");
        assert_eq!(body["rows"][0]["record_id"], "R-0200");

        // 排序：value_num 倒序，第一行应该是 249
        let res = handle(&req(
            "GET",
            "/api/table",
            &[
                ("path", path),
                ("table", "records"),
                ("limit", "3"),
                ("sort", "value_num:desc"),
            ],
            "",
        ));
        let body: Value = serde_json::from_str(&res.body).unwrap();
        assert_eq!(body["rows"][0]["value_num"], "249");
        assert_eq!(body["total"], 250, "排序不影响总数");

        // 筛选后的总数（total 是筛选后的口径）
        let res = handle(&req(
            "GET",
            "/api/table",
            &[
                ("path", path),
                ("table", "records"),
                ("limit", "10"),
                ("filter", "value_num:gt:240"),
            ],
            "",
        ));
        let body: Value = serde_json::from_str(&res.body).unwrap();
        assert_eq!(body["total"], 9, "241..249 共 9 条：{body}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    fn copy_dir(from: &std::path::Path, to: &std::path::Path) {
        std::fs::create_dir_all(to).unwrap();
        for e in std::fs::read_dir(from).unwrap().flatten() {
            let p = e.path();
            let dest = to.join(e.file_name());
            if p.is_dir() {
                copy_dir(&p, &dest);
            } else {
                std::fs::copy(&p, &dest).unwrap();
            }
        }
    }
}
