//! 操作系统、进程互斥、IPC 与安全隔离常量

/// Windows 单实例命名的互斥体标识
pub const SINGLE_INSTANCE_MUTEX: &str = "Local\\VProxy-7F61E55C-DB54-49C6-AB9E-986AEBFDD56B";

/// Windows 防火墙放行规则前缀
pub const FIREWALL_RULE_PREFIX: &str = "VProxy";

/// Windows 开机自启注册表键值名称
pub const LAUNCH_AT_STARTUP_VALUE_NAME: &str = "VProxy";

/// 品牌改名前（VPN Parser）的开机自启键值名：同步自启动时顺手清掉旧残留
pub const LEGACY_STARTUP_VALUE_NAME: &str = "VPN Parser";

/// Windows 系统托盘隐藏消息接收窗口类名
pub const TRAY_WINDOW_CLASS: &str = "VProxyTrayWindowClass";

/// 唤醒已存在实例的 Windows 自定义消息名
pub const WAKEUP_MSG_NAME: &str = "VPROXY_SHOW_WINDOW_MSG\0";

/// Unix 域套接字单实例文件名称
pub const UNIX_SOCKET_NAME: &str = "vproxy.sock";

/// 诊断模式环境变量名
pub const DIAG_ENV_VAR: &str = "VPROXY_DIAG";

/// 诊断实例屏蔽修改系统代理时的提示文案
pub const DIAG_BLOCKED_MSG: &str = "诊断实例（VPROXY_DIAG）不写系统代理与终端代理设置";
