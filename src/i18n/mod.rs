//! 多语言国际化 (i18n) 模块
//! 支持：简体中文 (zh-CN), 繁體中文 (zh-TW), English (en-US), 日本語 (ja-JP)

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Language {
    #[default]
    ZhCn,
    En,
    ZhTw,
    Ja,
}

impl Language {
    pub const ALL: [Self; 4] = [Self::ZhCn, Self::En, Self::ZhTw, Self::Ja];

    pub fn code(self) -> &'static str {
        match self {
            Self::ZhCn => "zh-CN",
            Self::En => "en-US",
            Self::ZhTw => "zh-TW",
            Self::Ja => "ja-JP",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::ZhCn => "简体中文",
            Self::En => "English",
            Self::ZhTw => "繁體中文",
            Self::Ja => "日本語",
        }
    }

    pub fn from_code(code: &str) -> Self {
        match code {
            "en" | "en-US" | "en_US" => Self::En,
            "zh-TW" | "zh-HK" | "zh_TW" | "zh_HK" | "zh-Hant" => Self::ZhTw,
            "ja" | "ja-JP" | "ja_JP" => Self::Ja,
            _ => Self::ZhCn,
        }
    }
}

/// 翻译字典核心函数
pub fn tr<'a>(lang: Language, key: &'a str) -> &'a str {
    match lang {
        Language::ZhCn => key,
        Language::En => match key {
            "控制台" => "Dashboard",
            "节点池" => "Node Pool",
            "订阅源" => "Subscriptions",
            "系统设置" => "Settings",
            "关于" => "About",
            "外部接口" => "API Service",
            "通用" => "General",
            "外观" => "Appearance",
            "订阅与测速" => "Automation",
            "自愈网络" => "Self-Healing",
            "新建" => "New",
            "拉取全部" => "Fetch All",
            "代理拉取全部" => "Proxy Fetch All",
            "导入订阅" => "Import",
            "导出订阅" => "Export",
            "清空全部" => "Clear All",
            "保存设置" => "Save Settings",
            "重置修改" => "Reset Changes",
            "已同步" => "Synced",
            "刷新" => "Refresh",
            "界面语言" => "Interface Language",
            "界面语言（即时生效并保存）" => "Interface Language (Saved automatically)",
            "界面配色主题" => "Color Theme",
            "夜间模式（深色底）" => "Dark Mode",
            "字体大小" => "Font Scale",
            "一键连接最优" => "Connect Best",
            "启动代理" => "Start Proxy",
            "停止代理" => "Stop Proxy",
            "系统代理" => "System Proxy",
            "TUN 模式" => "TUN Mode",
            "图表" => "Charts",
            "可用节点" => "Available Nodes",
            "分流策略组" => "Policy Groups",
            "活跃连接" => "Connections",
            "分流规则" => "Routing Rules",
            "DNS / TUN" => "DNS / TUN",
            "全选" => "Select All",
            "清空勾选" => "Clear Selection",
            "完成" => "Done",
            "关闭" => "Close",
            "取消" => "Cancel",
            "确定" => "OK",
            "订阅列表" => "Subscription List",
            "订阅拉取结果" => "Fetch Results",
            "接入 OpenRung 节点" => "Connect OpenRung",
            "原生注册 WARP 节点" => "Register WARP Native",
            "Gist 节点" => "Gist Nodes",
            "Gist 配置" => "Gist Config",
            "反向剔除同质化源" => "Prune Redundant Sources",
            "重置历史" => "Reset History",
            "深度整理内存" => "Deep Memory Trim",
            "Windows 防火墙放行" => "Windows Firewall Rules",
            "软件启动时自动加入防火墙放行规则" => "Auto-allow app on startup",
            "立即写入防火墙规则" => "Add Rules Now",
            "显示高级设置" => "Show Advanced Settings",
            _ => key,
        },
        Language::ZhTw => match key {
            "控制台" => "主控台",
            "节点池" => "節點池",
            "订阅源" => "訂閱源",
            "系统设置" => "系統設定",
            "关于" => "關於",
            "外部接口" => "外部介面",
            "通用" => "一般",
            "外观" => "外觀",
            "订阅与测速" => "訂閱與測速",
            "自愈网络" => "自癒網路",
            "新建" => "新增",
            "拉取全部" => "全部擷取",
            "代理拉取全部" => "透過代理擷取",
            "导入订阅" => "匯入訂閱",
            "导出订阅" => "匯出訂閱",
            "清空全部" => "清空全部",
            "保存设置" => "儲存設定",
            "重置修改" => "重設修改",
            "已同步" => "已同步",
            "刷新" => "重新整理",
            "界面语言" => "介面語言",
            "界面语言（即时生效并保存）" => "介面語言（即時生效並儲存）",
            "界面配色主题" => "配色主題",
            "夜间模式（深色底）" => "深色模式",
            "字体大小" => "字型大小",
            "一键连接最优" => "一鍵連線最佳",
            "启动代理" => "啟動代理",
            "停止代理" => "停止代理",
            "系统代理" => "系統代理",
            "TUN 模式" => "TUN 模式",
            "图表" => "圖表",
            "可用节点" => "可用節點",
            "分流策略组" => "分流策略組",
            "活跃连接" => "連線活動",
            "分流规则" => "分流規則",
            "DNS / TUN" => "DNS / TUN",
            "全选" => "全選",
            "清空勾选" => "清除選取",
            "完成" => "完成",
            "关闭" => "關閉",
            "取消" => "取消",
            "确定" => "確定",
            "订阅列表" => "訂閱清單",
            "订阅拉取结果" => "擷取結果",
            "接入 OpenRung 节点" => "接入 OpenRung 節點",
            "原生注册 WARP 节点" => "原生註冊 WARP 節點",
            "Gist 节点" => "Gist 節點",
            "Gist 配置" => "Gist 設定",
            "反向剔除同质化源" => "剔除同質冗餘源",
            "重置历史" => "重設歷史紀錄",
            "深度整理内存" => "深度整理記憶體",
            "Windows 防火墙放行" => "Windows 防火牆放行",
            "软件启动时自动加入防火墙放行规则" => "軟體啟動時自動加入允許規則",
            "立即写入防火墙规则" => "立即寫入規則",
            "显示高级设置" => "顯示進階設定",
            _ => key,
        },
        Language::Ja => match key {
            "控制台" => "ダッシュボード",
            "节点池" => "ノードプール",
            "订阅源" => "サブスクリプション",
            "系统设置" => "システム設定",
            "关于" => "バージョン情報",
            "外部接口" => "外部API",
            "通用" => "一般",
            "外观" => "外観",
            "订阅与测速" => "自動取得と速度測定",
            "自愈网络" => "セルフヒーリング",
            "新建" => "新規作成",
            "拉取全部" => "すべて取得",
            "代理拉取全部" => "プロキシ経由取得",
            "导入订阅" => "インポート",
            "导出订阅" => "エクスポート",
            "清空全部" => "すべてクリア",
            "保存设置" => "設定を保存",
            "重置修改" => "変更をリセット",
            "已同步" => "同期済み",
            "刷新" => "更新",
            "界面语言" => "表示言語",
            "界面语言（即时生效并保存）" => "表示言語（即時適用して保存）",
            "界面配色主题" => "カラーテーマ",
            "夜间模式（深色底）" => "ダークモード",
            "字体大小" => "フォントサイズ",
            "一键连接最优" => "最適ノードに接続",
            "启动代理" => "プロキシ起動",
            "停止代理" => "プロキシ停止",
            "系统代理" => "システムプロキシ",
            "TUN 模式" => "TUNモード",
            "图表" => "チャート",
            "可用节点" => "利用可能ノード",
            "分流策略组" => "ルーティンググループ",
            "活跃连接" => "アクティブ接続",
            "分流规则" => "ルーティングルール",
            "DNS / TUN" => "DNS / TUN",
            "全选" => "すべて選択",
            "清空勾选" => "選択をクリア",
            "完成" => "完了",
            "关闭" => "閉じる",
            "取消" => "キャンセル",
            "确定" => "確定",
            "订阅列表" => "登録一覧",
            "订阅拉取结果" => "取得結果",
            "接入 OpenRung 节点" => "OpenRung 接続",
            "原生注册 WARP 节点" => "WARP ネイティブ登録",
            "Gist 节点" => "Gist ノード",
            "Gist 配置" => "Gist 設定",
            "反向剔除同质化源" => "重複ソースを削除",
            "重置历史" => "履歴リセット",
            "深度整理内存" => "メモリ解放",
            "Windows 防火墙放行" => "Windows ファイアウォール許可",
            "软件启动时自动加入防火墙放行规则" => "起動時にファイアウォール規則を自動追加",
            "立即写入防火墙规则" => "今すぐ規則を追加",
            "显示高级设置" => "詳細設定を表示",
            _ => key,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_language_from_code() {
        assert_eq!(Language::from_code("zh-CN"), Language::ZhCn);
        assert_eq!(Language::from_code("en-US"), Language::En);
        assert_eq!(Language::from_code("en"), Language::En);
        assert_eq!(Language::from_code("zh-TW"), Language::ZhTw);
        assert_eq!(Language::from_code("ja-JP"), Language::Ja);
        assert_eq!(Language::from_code("unknown"), Language::ZhCn);
    }

    #[test]
    fn test_translations() {
        assert_eq!(tr(Language::ZhCn, "控制台"), "控制台");
        assert_eq!(tr(Language::En, "控制台"), "Dashboard");
        assert_eq!(tr(Language::ZhTw, "控制台"), "主控台");
        assert_eq!(tr(Language::Ja, "控制台"), "ダッシュボード");

        assert_eq!(tr(Language::En, "系统设置"), "Settings");
        assert_eq!(tr(Language::ZhTw, "系统设置"), "系統設定");
        assert_eq!(tr(Language::Ja, "系统设置"), "システム設定");
    }
}
