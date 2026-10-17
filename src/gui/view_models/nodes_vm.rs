#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeCardAction {
    Select { id: String, name: String },
    Connect { id: String, name: String },
    Speedtest(String),
    ShowDetail(String),
    CheckIpRisk(String),
    Delete(String),
}

#[derive(Debug, Default)]
pub struct NodesViewModel;

impl NodesViewModel {
    /// 计算分页总页数，最小为 1
    #[inline]
    pub fn total_pages(total: i64, page_size: u32) -> u32 {
        if page_size == 0 || total <= 0 {
            return 1;
        }
        ((total as f32) / (page_size as f32)).ceil().max(1.0) as u32
    }

    /// 根据容器宽度自适应计算网格列数与总卡片行数
    #[inline]
    pub fn grid_layout(available_width: f32, count: usize) -> (usize, usize) {
        let columns = if available_width >= 920.0 {
            4
        } else if available_width >= 660.0 {
            3
        } else {
            2
        };
        let rows = if count == 0 {
            0
        } else {
            (count + columns - 1) / columns
        };
        (columns, rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_total_pages_calculation() {
        assert_eq!(NodesViewModel::total_pages(0, 50), 1);
        assert_eq!(NodesViewModel::total_pages(49, 50), 1);
        assert_eq!(NodesViewModel::total_pages(50, 50), 1);
        assert_eq!(NodesViewModel::total_pages(51, 50), 2);
        assert_eq!(NodesViewModel::total_pages(100, 50), 2);
        assert_eq!(NodesViewModel::total_pages(101, 50), 3);
    }

    #[test]
    fn test_grid_layout_columns() {
        assert_eq!(NodesViewModel::grid_layout(1000.0, 10).0, 4);
        assert_eq!(NodesViewModel::grid_layout(700.0, 10).0, 3);
        assert_eq!(NodesViewModel::grid_layout(500.0, 10).0, 2);
    }
}
