# 界面自动验收报告

- 时间：2026-10-05 22:07:52 CST
- 内核：`/Users/wenhuan/PycharmProjects/Cercis/kernel/target/debug/cercis`
- app：`/Users/wenhuan/PycharmProjects/Cercis/apps/desktop/target/debug/cercis-app`
- 结果：**通过 24 ｜ 失败 0**
- 截图：6 张（同目录 .png，不入库）

## 这一轮验了什么

- 打开项目（界面里填路径 → 点「打开项目」）
- 数据表：能看到行、点表头排序、筛选（含被筛掉的行不显示）
- 待办：逾期/今天分桶、勾选完成后进「已完成」
- 历史：版本链 + 记录差异（生效值 + 更正理由）
- 搬运：试运行打包、共存体检
- 录入员：会话列表、草稿值、原音挂载
- 插件/插件市场/加密区域/工具页能打开；**浏览器控制台无报错**

> 复跑：`bash apps/desktop/ui-tests/run-ui-acceptance.sh`
> （自动造演示项目、起 app、开无头浏览器点一遍；不需要人工操作）
