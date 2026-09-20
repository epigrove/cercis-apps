//! 本地 API：界面与内核之间的通道。
//!
//! 所有数据操作都经内核（cercis-core），界面自己不碰文件——
//! 这正是文档里的硬规则：**app 是用户与内核之间的唯一中介**。
//!
//! 语言：所有面向用户的文字走 `lang` 参数（zh/en），中英双语。

use cercis_core::query::{Op, Query, SortDir};
use cercis_core::Project;
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
                    let mut tables = Vec::new();
                    for t in &template.tables {
                        let table_path = p.root().join("tables").join(format!("{}.csv", t.name));
                        let (rows, cols) = if table_path.exists() {
                            match cercis_core::Table::load(&table_path) {
                                Ok(tb) => (tb.rows.len(), tb.columns.clone()),
                                Err(_) => (0, vec![]),
                            }
                        } else {
                            (0, vec![])
                        };
                        tables.push(json!({
                            "name": t.name,
                            "label": if lang == Lang::Zh { &t.label_cn } else { &t.label_en },
                            "primary_key": t.primary_key,
                            "rows": rows,
                            "columns": cols,
                        }));
                    }
                    Response::json(
                        &json!({
                            "ok": true,
                            "project": p.config.name,
                            "root": p.root().display().to_string(),
                            "template": template.name,
                            "language": if lang == Lang::Zh { "zh" } else { "en" },
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
                .unwrap_or(100);

            match Project::open(path, None) {
                Ok(p) => {
                    let mut q = Query::new().page(0, limit);
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

                    match p.query(table, q) {
                        Ok(rows) => {
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
                                    "rows": rows,
                                })
                                .to_string(),
                            )
                        }
                        Err(e) => Response::error(400, &e.to_string()),
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

        // 插件列表（含信任状态）
        "/api/plugins" => {
            let Some(path) = req.query.get("path") else {
                return Response::error(400, lang.t("缺少 path 参数", "missing path parameter"));
            };
            match cercis_core::plugin::discover(path, crate::plugin_host::global_plugins_dir().as_deref()) {
                Ok(found) => {
                    let trust = cercis_core::TrustRecord::load(path).unwrap_or_default();
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

        _ => Response::error(404, "unknown endpoint"),
    }
}
