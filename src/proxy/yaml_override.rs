use anyhow::Result;
use serde_json::Value;

/// Clash Party 规范中键名修饰符类型
/// 参见: <https://clashparty.org/docs/guide/override/yaml>
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyModifier {
    /// 默认修饰：
    /// - 简单值（字符串、数字、布尔等）：直接覆盖
    /// - 嵌套对象（Object / Mapping）：递归深度合并子字段
    /// - 列表数组（Array / Sequence）：直接替换整组原列表
    Normal,

    /// `!` 修饰符（强制覆盖）：
    /// 无论目标是简单值还是嵌套对象，强制用新值替换整个属性，放弃递归合并
    ForceReplace,

    /// `+` 前缀修饰符（前置插入）：
    /// 将新列表元素插入到原数组前面（如 `+rules:`）
    Prepend,

    /// `+` 后缀修饰符（后置追加）：
    /// 将新列表元素追加到原数组末尾（如 `rules+:`）
    Append,
}

/// 解析后的键名结构
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedKey {
    /// 实际键名（已剥离修饰符与 `<>` 转义符）
    pub name: String,
    /// 该键名对应的修饰符
    pub modifier: KeyModifier,
}

/// 解析包含修饰符或 `<...>` 转义符的键名
///
/// 规则：
/// 1. 若键名以 `<` 和 `>` 包裹转义：
///    - `+<name>` -> Prepend, 实际键为 `name`
///    - `<name>+` -> Append, 实际键为 `name`
///    - `<name>!` -> ForceReplace, 实际键为 `name`
///    - `<name>`  -> Normal, 实际键为 `name`
/// 2. 无 `<>` 转义时的常规修饰：
///    - 键名以 `!` 结尾（长度 > 1）：ForceReplace（如 `dns!`）
///    - 键名以 `+` 开头（长度 > 1）：Prepend（如 `+rules`）
///    - 键名以 `+` 结尾（长度 > 1）：Append（如 `rules+`）
///    - 其它情况：Normal
pub fn parse_override_key(raw_key: &str) -> ParsedKey {
    let key = raw_key.trim();
    if key.is_empty() {
        return ParsedKey {
            name: String::new(),
            modifier: KeyModifier::Normal,
        };
    }

    // 1. 处理以 `+<...>` 开头的前置转义
    if key.starts_with("+<") && key.ends_with('>') && key.len() > 3 {
        let inner = &key[2..key.len() - 1];
        return ParsedKey {
            name: inner.to_string(),
            modifier: KeyModifier::Prepend,
        };
    }

    // 2. 处理以 `<...>` 开头的转义
    if key.starts_with('<') {
        if let Some(close_idx) = key.find('>') {
            let inner = &key[1..close_idx];
            let rest = &key[close_idx + 1..];
            if rest == "+" {
                return ParsedKey {
                    name: inner.to_string(),
                    modifier: KeyModifier::Append,
                };
            }
            if rest == "!" {
                return ParsedKey {
                    name: inner.to_string(),
                    modifier: KeyModifier::ForceReplace,
                };
            }
            if rest.is_empty() {
                return ParsedKey {
                    name: inner.to_string(),
                    modifier: KeyModifier::Normal,
                };
            }
        }
    }

    // 3. 常规无转义修饰符
    if key.len() > 1 {
        if let Some(stripped) = key.strip_suffix('!') {
            return ParsedKey {
                name: stripped.to_string(),
                modifier: KeyModifier::ForceReplace,
            };
        }
        if let Some(stripped) = key.strip_prefix('+') {
            return ParsedKey {
                name: stripped.to_string(),
                modifier: KeyModifier::Prepend,
            };
        }
        if let Some(stripped) = key.strip_suffix('+') {
            return ParsedKey {
                name: stripped.to_string(),
                modifier: KeyModifier::Append,
            };
        }
    }

    ParsedKey {
        name: key.to_string(),
        modifier: KeyModifier::Normal,
    }
}

/// 合并数组：当数组元素包含具名对象（含 `name` 字段，如策略组）时，
/// 自动剔除 target 中被 patch 同名覆盖的旧项，杜绝重名报错。
fn merge_arrays(target_arr: &[Value], patch_arr: &[Value], modifier: KeyModifier) -> Vec<Value> {
    match modifier {
        KeyModifier::Prepend => {
            let patch_names: std::collections::HashSet<&str> = patch_arr
                .iter()
                .filter_map(|v| v.get("name").and_then(Value::as_str))
                .collect();
            let filtered_target: Vec<Value> = target_arr
                .iter()
                .filter(|v| {
                    if let Some(name) = v.get("name").and_then(Value::as_str) {
                        !patch_names.contains(name)
                    } else {
                        true
                    }
                })
                .cloned()
                .collect();
            let mut res = patch_arr.to_vec();
            res.extend(filtered_target);
            res
        }
        KeyModifier::Append => {
            let patch_names: std::collections::HashSet<&str> = patch_arr
                .iter()
                .filter_map(|v| v.get("name").and_then(Value::as_str))
                .collect();
            let mut res: Vec<Value> = target_arr
                .iter()
                .filter(|v| {
                    if let Some(name) = v.get("name").and_then(Value::as_str) {
                        !patch_names.contains(name)
                    } else {
                        true
                    }
                })
                .cloned()
                .collect();
            res.extend(patch_arr.iter().cloned());
            res
        }
        KeyModifier::Normal | KeyModifier::ForceReplace => patch_arr.to_vec(),
    }
}

/// 按照 Clash Party 规范对目标 JSON/YAML 树进行深度合并
///
/// 算法规则：
/// - 简单类型（字符串、数字、布尔、null）：直接覆盖原值；
/// - 嵌套对象（Object / Mapping）：默认递归逐字段合并，带有 `!` 时强制全量替换；
/// - 数组（Array / Sequence）：
///   - 默认（常规键名）：直接替换整组原数组；
///   - `+key`：新数组元素插入到原数组前面；
///   - `key+`：新数组元素追加到原数组末尾。
pub fn deep_merge_value(target: &mut Value, patch: Value) {
    match patch {
        Value::Object(patch_obj) => {
            if !target.is_object() {
                *target = Value::Object(Default::default());
            }
            let target_obj = target.as_object_mut().expect("target must be an object");

            for (raw_key, patch_val) in patch_obj {
                let parsed = parse_override_key(&raw_key);
                let key_name = parsed.name;

                if !target_obj.contains_key(&key_name) {
                    target_obj.insert(key_name, patch_val);
                    continue;
                }

                let target_val = target_obj.get_mut(&key_name).expect("key must exist");

                // 强制覆盖修饰符 `!`：不论是什么类型，直接全量替换，不进行递归合并
                if parsed.modifier == KeyModifier::ForceReplace {
                    *target_val = patch_val;
                    continue;
                }

                // 数组类型处理
                if target_val.is_array() && patch_val.is_array() {
                    let t_arr = target_val.as_array().map(|v| v.as_slice()).unwrap_or(&[]);
                    let p_arr = patch_val.as_array().map(|v| v.as_slice()).unwrap_or(&[]);
                    let merged = merge_arrays(t_arr, p_arr, parsed.modifier);
                    *target_val = Value::Array(merged);
                    continue;
                }

                // 嵌套对象：递归深度合并
                if target_val.is_object() && patch_val.is_object() {
                    deep_merge_value(target_val, patch_val);
                    continue;
                }

                // 其它类型（如简单值，或类型不一致）：直接替换
                *target_val = patch_val;
            }
        }
        other => {
            *target = other;
        }
    }
}

/// 解析用户提供的 YAML 覆写文本并深度合并到目标配置中
#[allow(dead_code)]
pub fn apply_yaml_override(target: &mut Value, override_yaml: &str) -> Result<()> {
    let trimmed = override_yaml.trim();
    if trimmed.is_empty() {
        return Ok(());
    }

    let patch: Value = serde_yaml::from_str(trimmed)
        .map_err(|e| anyhow::anyhow!("解析自定义 YAML 覆写失败: {e}"))?;

    deep_merge_value(target, patch);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_parse_override_key_modifiers() {
        assert_eq!(
            parse_override_key("mixed-port"),
            ParsedKey {
                name: "mixed-port".to_string(),
                modifier: KeyModifier::Normal,
            }
        );
        assert_eq!(
            parse_override_key("+rules"),
            ParsedKey {
                name: "rules".to_string(),
                modifier: KeyModifier::Prepend,
            }
        );
        assert_eq!(
            parse_override_key("rules+"),
            ParsedKey {
                name: "rules".to_string(),
                modifier: KeyModifier::Append,
            }
        );
        assert_eq!(
            parse_override_key("dns!"),
            ParsedKey {
                name: "dns".to_string(),
                modifier: KeyModifier::ForceReplace,
            }
        );
    }

    #[test]
    fn test_parse_override_key_escape_brackets() {
        assert_eq!(
            parse_override_key("<+.google.cn>"),
            ParsedKey {
                name: "+.google.cn".to_string(),
                modifier: KeyModifier::Normal,
            }
        );
        assert_eq!(
            parse_override_key("+<+.google.cn>"),
            ParsedKey {
                name: "+.google.cn".to_string(),
                modifier: KeyModifier::Prepend,
            }
        );
        assert_eq!(
            parse_override_key("<+.google.cn>+"),
            ParsedKey {
                name: "+.google.cn".to_string(),
                modifier: KeyModifier::Append,
            }
        );
        assert_eq!(
            parse_override_key("<custom!>!"),
            ParsedKey {
                name: "custom!".to_string(),
                modifier: KeyModifier::ForceReplace,
            }
        );
    }

    #[test]
    fn test_deep_merge_primitive_overwrite() {
        let mut target = json!({
            "mixed-port": 7890,
            "mode": "rule"
        });
        let patch = json!({
            "mixed-port": 7892,
            "allow-lan": true
        });
        deep_merge_value(&mut target, patch);
        assert_eq!(target["mixed-port"], 7892);
        assert_eq!(target["mode"], "rule");
        assert_eq!(target["allow-lan"], true);
    }

    #[test]
    fn test_deep_merge_nested_object_recursive() {
        let mut target = json!({
            "dns": {
                "enable": true,
                "enhanced-mode": "fake-ip",
                "nameserver": ["223.5.5.5"]
            }
        });
        let patch = json!({
            "dns": {
                "enhanced-mode": "redir-host",
                "listen": "0.0.0.0:53"
            }
        });
        deep_merge_value(&mut target, patch);
        assert_eq!(target["dns"]["enable"], true);
        assert_eq!(target["dns"]["enhanced-mode"], "redir-host");
        assert_eq!(target["dns"]["listen"], "0.0.0.0:53");
        assert_eq!(target["dns"]["nameserver"], json!(["223.5.5.5"]));
    }

    #[test]
    fn test_deep_merge_force_replace_object() {
        let mut target = json!({
            "dns": {
                "enable": true,
                "enhanced-mode": "fake-ip",
                "nameserver": ["223.5.5.5"]
            }
        });
        let patch = json!({
            "dns!": {
                "enable": false
            }
        });
        deep_merge_value(&mut target, patch);
        assert_eq!(target["dns"], json!({ "enable": false }));
    }

    #[test]
    fn test_deep_merge_array_operations() {
        // 1. 常规替换
        let mut target = json!({
            "rules": ["B", "C"]
        });
        let patch = json!({
            "rules": ["X", "Y"]
        });
        deep_merge_value(&mut target, patch);
        assert_eq!(target["rules"], json!(["X", "Y"]));

        // 2. 前置插入 (+key)
        let mut target = json!({
            "rules": ["B", "C"]
        });
        let patch = json!({
            "+rules": ["A"]
        });
        deep_merge_value(&mut target, patch);
        assert_eq!(target["rules"], json!(["A", "B", "C"]));

        // 3. 后置追加 (key+)
        let mut target = json!({
            "rules": ["A", "B"]
        });
        let patch = json!({
            "rules+": ["C", "D"]
        });
        deep_merge_value(&mut target, patch);
        assert_eq!(target["rules"], json!(["A", "B", "C", "D"]));

        // 4. 具名对象（如策略组）前置去重
        let mut target = json!({
            "proxy-groups": [
                { "name": "group1", "type": "select" },
                { "name": "group2", "type": "select" }
            ]
        });
        let patch = json!({
            "+proxy-groups": [
                { "name": "group1", "type": "url-test" },
                { "name": "group3", "type": "select" }
            ]
        });
        deep_merge_value(&mut target, patch);
        assert_eq!(
            target["proxy-groups"],
            json!([
                { "name": "group1", "type": "url-test" },
                { "name": "group3", "type": "select" },
                { "name": "group2", "type": "select" }
            ])
        );
    }

    #[test]
    fn test_apply_yaml_override_full() {
        let mut target = json!({
            "mixed-port": 7890,
            "dns": {
                "enable": true,
                "nameserver": ["223.5.5.5"]
            },
            "rules": [
                "GEOIP,CN,DIRECT",
                "MATCH,PROXY"
            ]
        });

        let yaml = r#"
mixed-port: 7895
dns:
  +nameserver:
    - 1.1.1.1
+rules:
  - DOMAIN-SUFFIX,openai.com,AI分组
rules+:
  - FINAL,DIRECT
tun:
  enable: true
  stack: gvisor
"#;

        apply_yaml_override(&mut target, yaml).unwrap();

        assert_eq!(target["mixed-port"], 7895);
        assert_eq!(target["dns"]["enable"], true);
        assert_eq!(target["dns"]["nameserver"], json!(["1.1.1.1", "223.5.5.5"]));
        assert_eq!(target["tun"]["enable"], true);
        assert_eq!(target["tun"]["stack"], "gvisor");
        assert_eq!(
            target["rules"],
            json!([
                "DOMAIN-SUFFIX,openai.com,AI分组",
                "GEOIP,CN,DIRECT",
                "MATCH,PROXY",
                "FINAL,DIRECT"
            ])
        );
    }
}
