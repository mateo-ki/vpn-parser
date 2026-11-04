//! 品牌、产品与全局元数据常量

/// 应用程序正式名称
pub const APP_NAME: &str = "VProxy";

/// 应用程序通用系统 ID (Wayland / X11 / 进程标记)
pub const APP_ID: &str = "vproxy";

/// Windows 可执行文件名
pub const APP_EXE_NAME: &str = "vproxy.exe";

/// 历史兼容名称
pub const LEGACY_APP_NAME: &str = "VPN Parser";

/// 历史兼容可执行文件名
pub const LEGACY_APP_EXE_NAME: &str = "vpn-parser.exe";

/// GitHub 仓库标识 (owner/repo)
pub const REPOSITORY: &str = "mateo-ki/vproxy";

/// 项目开源主页 URL
pub const REPO_URL: &str = "https://github.com/mateo-ki/vproxy";

/// 发行版 Releases 页面 URL
pub const RELEASES_URL: &str = "https://github.com/mateo-ki/vproxy/releases";

/// 关于页主打标语
pub const TAGLINE: &str = "采集订阅 · 解析节点 · 本地跑代理 · 把代理质量测出来给你看。";

/// 默认数据库文件名
pub const DEFAULT_DB_FILE: &str = "vproxy.db";

/// 历史兼容数据库文件名
pub const LEGACY_DB_FILE: &str = "vpn-parser.db";

/// 统一配置文件默认备份描述
pub const DEFAULT_PROFILE_BACKUP_NAME: &str = "VProxy 配置备份";

/// 统一配置文件默认文件名
pub const DEFAULT_PROFILE_FILE_NAME: &str = "vproxy-profile.yaml";

/// 默认 UI 主题 ID
pub const DEFAULT_THEME: &str = "theme-champagne";

/// SHA-256 校验文件名
pub const HASH_INDEX_NAME: &str = "sha256.txt";
