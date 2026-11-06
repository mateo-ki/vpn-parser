use anyhow::{anyhow, Result};
use serde_json::Value;

/// 创建经过安全沙箱强化的 Rhai 脚本执行引擎
/// 限制执行指令总数、字符串与集合大小以及表达式深度，严防死循环与内存溢出
fn create_sandboxed_engine() -> rhai::Engine {
    let mut engine = rhai::Engine::new();
    // 限制单次执行最大指令步数（约 1~3ms 耗时上限，彻底杜绝 while true 死循环卡死 GUI）
    engine.set_max_operations(100_000);
    // 限制单字符串最大长度 (50KB)
    engine.set_max_string_size(50_000);
    // 限制数组最大元素数 (10,000)
    engine.set_max_array_size(10_000);
    // 限制哈希表最大条目目数 (10,000)
    engine.set_max_map_size(10_000);
    // 限制表达式与递归深度 (50)，防止栈溢出崩溃
    engine.set_max_expr_depths(50, 50);
    engine
}

/// 快速语法校验：用于 GUI 编辑器实时测试脚本语法是否合规
pub fn validate_script_syntax(script: &str) -> Result<(), String> {
    let trimmed = script.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    let engine = create_sandboxed_engine();
    engine
        .compile(trimmed)
        .map(|_| ())
        .map_err(|e| format!("脚本语法解析错误: {e}"))
}

/// 执行用户自定义 Rhai 脚本，动态覆写内核配置树
///
/// 支持两种写法：
/// 1. 标准函数式写法（类似 Clash Verge/Party）：
///    ```rhai
///    fn main(config) {
///        config.dns.enhanced_mode = "fake-ip";
///        return config;
///    }
///    ```
/// 2. 扁平脚本式写法（无需写 fn main，直接操作全局预置的 `config` 变量）：
///    ```rhai
///    config.dns.enhanced_mode = "fake-ip";
///    config.rules.insert(0, "DOMAIN-SUFFIX,openai.com,AI分组");
///    ```
pub fn apply_script_override(config: &mut Value, script: &str) -> Result<()> {
    let trimmed = script.trim();
    if trimmed.is_empty() {
        return Ok(());
    }

    let engine = create_sandboxed_engine();
    let ast = engine
        .compile(trimmed)
        .map_err(|e| anyhow!("Rhai 脚本编译失败: {e}"))?;

    // 1. 将 serde_json::Value 转为 Rhai 动态变量 Dynamic
    let dynamic_config = rhai::serde::to_dynamic(&*config)
        .map_err(|e| anyhow!("配置转换为脚本动态变量失败: {e}"))?;

    let mut scope = rhai::Scope::new();

    // 2. 判定脚本中是否定义了 main(config) 函数
    let result_dynamic: rhai::Dynamic = if ast.iter_functions().any(|f| f.name == "main") {
        // 调用 main(config) 函数
        engine
            .call_fn(&mut scope, &ast, "main", (dynamic_config,))
            .map_err(|e| anyhow!("执行 main(config) 脚本失败: {e}"))?
    } else {
        // 扁平语句模式：将 config 注入作用域并执行整个 AST 表达式
        scope.push("config", dynamic_config);
        engine
            .run_ast_with_scope(&mut scope, &ast)
            .map_err(|e| anyhow!("执行 Rhai 脚本语句失败: {e}"))?;
        scope
            .get_value::<rhai::Dynamic>("config")
            .ok_or_else(|| anyhow!("脚本执行完毕后未能检索到 config 变量"))?
    };

    // 3. 将执行完毕返回的 Dynamic 反序列化回 serde_json::Value
    // `fn main(config) {}` 忘写 `return config;` 时 Rhai 返回 unit：直接交给 serde
    // 会炸出一句天书。这里前置拦住，把修复方法直接告诉用户。
    if result_dynamic.is_unit() {
        return Err(anyhow!(
            "脚本没有返回配置：fn main(config) 必须以 `return config;` 结尾（或改用直接操作 config 变量的扁平语句写法）"
        ));
    }
    let new_config: Value = rhai::serde::from_dynamic(&result_dynamic)
        .map_err(|e| anyhow!("脚本返回对象转换回内核配置失败: {e}"))?;

    *config = new_config;
    Ok(())
}

/// 默认内置的经典 Rhai 脚本模板
pub const RHAI_RECOMMENDED_SCRIPT_TEMPLATE: &str = r#"// =========================================================
// Rhai 动态配置覆写脚本模板
// =========================================================
fn main(config) {
    // 1. 动态定制 DNS 解析引擎
    config.dns.enhanced_mode = "fake-ip";

    // 2. 在规则最前排插入高优先级分流规则
    config.rules.insert(0, "DOMAIN-SUFFIX,openai.com,AI分组");
    config.rules.insert(1, "DOMAIN-KEYWORD,anthropic,AI分组");

    // 3. 编程式过滤节点列表（示例：排除名称中含 2x/3x 倍率的节点）
    let filtered_proxies = [];
    for p in config.proxies {
        if !p.name.contains("2x") && !p.name.contains("3x") {
            filtered_proxies.push(p);
        }
    }
    // 如果过滤后不为空，则更新节点池
    if filtered_proxies.len() > 0 {
        config.proxies = filtered_proxies;
    }

    return config;
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_script_override_fn_main() {
        let mut config = json!({
            "mixed-port": 7890,
            "dns": {
                "enable": true,
                "enhanced_mode": "redir-host"
            },
            "rules": [
                "GEOIP,CN,DIRECT",
                "MATCH,PROXY"
            ],
            "proxies": [
                { "name": "香港 01", "type": "ss" },
                { "name": "日本 02 [2x]", "type": "ss" },
                { "name": "美国 03", "type": "ss" }
            ]
        });

        let script = r#"
fn main(config) {
    config.mixed_port = 8888;
    config.dns.enhanced_mode = "fake-ip";
    config.rules.insert(0, "DOMAIN-SUFFIX,google.com,PROXY");

    let clean = [];
    for p in config.proxies {
        if !p.name.contains("2x") {
            clean.push(p);
        }
    }
    config.proxies = clean;
    return config;
}
"#;

        apply_script_override(&mut config, script).expect("script should succeed");

        assert_eq!(config["mixed_port"], 8888);
        assert_eq!(config["dns"]["enhanced_mode"], "fake-ip");
        assert_eq!(config["rules"][0], "DOMAIN-SUFFIX,google.com,PROXY");
        assert_eq!(config["rules"][1], "GEOIP,CN,DIRECT");
        let proxies = config["proxies"].as_array().unwrap();
        assert_eq!(proxies.len(), 2);
        assert_eq!(proxies[0]["name"], "香港 01");
        assert_eq!(proxies[1]["name"], "美国 03");
    }

    #[test]
    fn test_script_override_flat_statements() {
        let mut config = json!({
            "mixed-port": 7890,
            "tun": {
                "enable": false
            }
        });

        let script = r#"
config.mixed_port = 9090;
config.tun.enable = true;
config.tun.stack = "gvisor";
"#;

        apply_script_override(&mut config, script).expect("flat script should succeed");

        assert_eq!(config["mixed_port"], 9090);
        assert_eq!(config["tun"]["enable"], true);
        assert_eq!(config["tun"]["stack"], "gvisor");
    }

    #[test]
    fn test_script_syntax_validator() {
        assert!(validate_script_syntax("fn main(config) { return config; }").is_ok());
        assert!(validate_script_syntax("config.a = 1;").is_ok());
        assert!(validate_script_syntax("").is_ok());
        // 括号不匹配语法错误
        assert!(validate_script_syntax("fn main(config { return config; }").is_err());
    }

    #[test]
    fn test_script_watchdog_prevents_infinite_loop() {
        let mut config = json!({ "count": 0 });
        let infinite_loop_script = r#"
fn main(config) {
    while true {
        config.count += 1;
    }
    return config;
}
"#;
        let res = apply_script_override(&mut config, infinite_loop_script);
        assert!(res.is_err(), "死循环脚本必须被看门狗安全中断");
        let err_str = res.unwrap_err().to_string();
        assert!(
            err_str.contains("Too many operations") || err_str.contains("operations"),
            "错误信息中应明确包含超步数看门狗提示: {err_str}"
        );
    }

    #[test]
    fn test_script_missing_return_gives_actionable_error() {
        let mut config = json!({ "mixed-port": 7890 });
        // fn main 里改了字段但忘写 return config; —— Rhai 返回 unit，
        // 必须报"如何修"而不是 serde 天书。
        let err = apply_script_override(&mut config, "fn main(config) { config.mixed_port = 9090; }")
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("return config"),
            "缺少 return 的报错必须指引补上 return config;: {err}"
        );
        assert_eq!(config["mixed-port"], 7890, "失败时不得改动原配置");

        // 扁平语句写法没有 return 也合法：config 变量在作用域里直接生效。
        let mut config = json!({ "mixed-port": 7890 });
        apply_script_override(&mut config, "config.mixed_port = 9090;").unwrap();
        assert_eq!(config["mixed_port"], 9090);
    }
}
