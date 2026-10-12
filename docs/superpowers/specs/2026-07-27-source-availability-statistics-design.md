# 订阅源可用性统计设计

## 目标

在订阅源管理界面和 `GET /api/sources` 中展示每个订阅源当前归属节点的总数、真实可用数、不可用数和未测速数。

## 统计口径

- 总数：当前 `nodes.source_id` 等于订阅源 ID 的节点数。
- 可用：真实代理验证成功，即 `latency_ok = 1` 且 `latency_test_kind = real_proxy`。
- 不可用：真实代理验证失败，即 `latency_ok = 0` 且 `latency_test_kind = real_proxy`。
- 未测速：没有真实代理结论，包括 `latency_ok IS NULL`、历史 `tcp_only` 结果和未知测试类型。
- 恒等式：`总数 = 可用 + 不可用 + 未测速`。

## 数据查询

- 在 `Database::list_sources` 中通过一次 `LEFT JOIN` 聚合所有源，禁止 GUI 发起逐源 N+1 查询。
- 使用当前节点表实时统计，不使用 `sources.last_node_count` 作为当前总数。
- 复用现有 `idx_nodes_source`，不新增统计索引。
- 无节点的订阅源四项统计均返回 `0`。

## 模型与 API

在 `Source` 增加四个只读派生字段：

- `current_node_count`
- `available_node_count`
- `unavailable_node_count`
- `untested_node_count`

`GET /api/sources` 继续直接序列化 `Source`，无需新增接口。

## GUI

订阅源列表中的节点统计使用紧凑三行布局：

```text
总 88
可用 20 · 不可用 60
未测速 8
```

最近拉取解析量继续保留为独立信息，不再冒充当前节点总数。

## 数据限制

节点目前只有一个 `source_id`。跨源重复节点按当前归属源计数，不表示该节点曾在哪些源中出现。本阶段不新增订阅源与节点的多对多关系表。

## 测试

- 同一订阅源包含真实成功、真实失败、TCP-only 和未测速节点时统计正确。
- 无节点订阅源返回四个零。
- 四项满足总数恒等式。
- API 序列化包含新增字段。

