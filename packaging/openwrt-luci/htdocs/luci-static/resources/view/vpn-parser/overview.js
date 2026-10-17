'use strict';
'require view';
'require ui';
'require rpc';
'require poll';

var callGetStatus = rpc.declare({
	object: 'luci.vpn-parser',
	method: 'get_status'
});

var callGetStats = rpc.declare({
	object: 'luci.vpn-parser',
	method: 'get_stats'
});

var callGetExportLinks = rpc.declare({
	object: 'luci.vpn-parser',
	method: 'get_export_links',
	params: ['host']
});

var callSpeedtest = rpc.declare({
	object: 'luci.vpn-parser',
	method: 'speedtest',
	params: ['ids']
});

var callFetchAll = rpc.declare({
	object: 'luci.vpn-parser',
	method: 'fetch_all'
});

var callCleanNodes = rpc.declare({
	object: 'luci.vpn-parser',
	method: 'clean_nodes',
	params: ['action']
});

var callGetTasks = rpc.declare({
	object: 'luci.vpn-parser',
	method: 'get_tasks',
	params: ['limit']
});

return view.extend({
	load: function() {
		var host = window.location.hostname || '127.0.0.1';
		return Promise.all([
			callGetStatus(),
			callGetStats(),
			callGetExportLinks(host),
			callGetTasks(5)
		]);
	},

	render: function(data) {
		var status = data[0] || {};
		var stats = (data[1] && data[1].data) ? data[1].data : (data[1] || {});
		var links = data[2] || {};
		var tasks = (data[3] && data[3].data) ? data[3].data : [];

		var isRunning = status.running === true;

		var viewRoot = E('div', { 'class': 'cbi-map' }, [
			E('h2', {}, _('VPN Parser - 服务概览')),
			E('div', { 'class': 'cbi-map-descr' }, _('高性能订阅采集清洗、真握手并发测速与纯净节点订阅微服务'))
		]);

		// 1. 服务运行状态卡片
		var statusBadge = isRunning
			? E('span', { 'class': 'badge', 'style': 'background-color:#2ecc71; color:#fff; padding:4px 10px; border-radius:12px; font-weight:bold;' }, '🟢 运行中 (PID: ' + status.pid + ')')
			: E('span', { 'class': 'badge', 'style': 'background-color:#e74c3c; color:#fff; padding:4px 10px; border-radius:12px; font-weight:bold;' }, '🔴 已停止');

		var statusSection = E('div', { 'class': 'cbi-section' }, [
			E('h3', {}, _('运行状态')),
			E('div', { 'style': 'display:flex; align-items:center; gap:16px; margin:12px 0;' }, [
				statusBadge,
				E('span', {}, 'API 监听端口: ' + (status.port || 8080)),
				E('button', {
					'class': 'cbi-button cbi-button-action',
					'click': function(ev) {
						window.location.reload();
					}
				}, '🔄 刷新状态')
			])
		]);
		viewRoot.appendChild(statusSection);

		// 2. 数据看板卡片 (统计栅格)
		var totalNodes = stats.node_count || 0;
		var aliveNodes = stats.alive_count || 0;
		var totalSources = stats.source_count || 0;
		var enabledSources = stats.enabled_source_count || 0;
		var aliveRate = totalNodes > 0 ? ((aliveNodes / totalNodes) * 100).toFixed(1) : '0.0';

		var metricsRow = E('div', { 'style': 'display:grid; grid-template-columns: repeat(auto-fit, minmax(180px, 1fr)); gap:16px; margin-bottom:16px;' }, [
			E('div', { 'class': 'cbi-value', 'style': 'padding:14px; background:var(--cbi-section-bg, #f8f9fa); border-radius:8px; border:1px solid #e0e0e0;' }, [
				E('div', { 'style': 'color:#666; font-size:12px;' }, '已启用订阅源 / 总数'),
				E('div', { 'style': 'font-size:24px; font-weight:bold; color:#2980b9; margin-top:4px;' }, enabledSources + ' / ' + totalSources)
			]),
			E('div', { 'class': 'cbi-value', 'style': 'padding:14px; background:var(--cbi-section-bg, #f8f9fa); border-radius:8px; border:1px solid #e0e0e0;' }, [
				E('div', { 'style': 'color:#666; font-size:12px;' }, '可用活节点数 / 总数'),
				E('div', { 'style': 'font-size:24px; font-weight:bold; color:#27ae60; margin-top:4px;' }, aliveNodes + ' / ' + totalNodes)
			]),
			E('div', { 'class': 'cbi-value', 'style': 'padding:14px; background:var(--cbi-section-bg, #f8f9fa); border-radius:8px; border:1px solid #e0e0e0;' }, [
				E('div', { 'style': 'color:#666; font-size:12px;' }, '节点整体可用率'),
				E('div', { 'style': 'font-size:24px; font-weight:bold; color:#d35400; margin-top:4px;' }, aliveRate + '%')
			])
		]);

		// 协议分布药丸
		var protoChips = E('div', { 'style': 'display:flex; flex-wrap:wrap; gap:8px; margin-top:10px;' });
		if (stats.protocol_stats && stats.protocol_stats.length > 0) {
			stats.protocol_stats.forEach(function(p) {
				protoChips.appendChild(E('span', {
					'style': 'background:#eef2f7; color:#333; padding:3px 10px; border-radius:12px; font-size:12px; font-weight:600;'
				}, p.protocol.toUpperCase() + ': ' + p.count));
			});
		}

		var statsSection = E('div', { 'class': 'cbi-section' }, [
			E('h3', {}, _('节点池数据看板')),
			metricsRow,
			E('div', { 'style': 'margin-top:8px;' }, [
				E('span', { 'style': 'font-size:13px; font-weight:600; color:#555;' }, '协议分布: '),
				protoChips
			])
		]);
		viewRoot.appendChild(statsSection);

		// 3. 核心快捷操作栏
		var actionSection = E('div', { 'class': 'cbi-section' }, [
			E('h3', {}, _('快捷运维操作')),
			E('div', { 'style': 'display:flex; flex-wrap:wrap; gap:12px; margin-top:10px;' }, [
				E('button', {
					'class': 'cbi-button cbi-button-apply',
					'click': function(ev) {
						ui.showModal(_('发起全量测速'), [
							E('p', {}, _('正在启动后台并发真握手测速任务...')),
							E('div', { 'class': 'cbi-page-actions' }, [
								E('button', {
									'class': 'cbi-button cbi-button-action',
									'click': function() { ui.hideModal(); window.location.reload(); }
								}, _('确定'))
							])
						]);
						callSpeedtest([]).catch(function(err) {
							ui.addNotification(null, E('p', {}, '启动测速失败: ' + err), 'error');
						});
					}
				}, '⚡ 测速全部节点'),
				E('button', {
					'class': 'cbi-button cbi-button-action',
					'click': function(ev) {
						ui.addNotification(null, E('p', {}, '已触发拉取全部已启用订阅源...'), 'info');
						callFetchAll().then(function() {
							ui.addNotification(null, E('p', {}, '订阅源拉取完成！'), 'success');
							window.location.reload();
						});
					}
				}, '🔄 拉取全部订阅'),
				E('button', {
					'class': 'cbi-button cbi-button-remove',
					'click': function(ev) {
						if (!confirm('确定清理所有超时失效的死节点吗？')) return;
						callCleanNodes('delete_dead').then(function() {
							ui.addNotification(null, E('p', {}, '死节点已清理！'), 'success');
							window.location.reload();
						});
					}
				}, '🧹 清理失效死节点'),
				E('button', {
					'class': 'cbi-button cbi-button-neutral',
					'click': function(ev) {
						callCleanNodes('dedupe').then(function() {
							ui.addNotification(null, E('p', {}, '物理去重完成！'), 'success');
							window.location.reload();
						});
					}
				}, '🔀 物理去重')
			])
		]);
		viewRoot.appendChild(actionSection);

		// 4. 下游插件联动专区 (一键复制订阅)
		function makeCopyRow(label, url) {
			var input = E('input', {
				'type': 'text',
				'class': 'cbi-input-text',
				'style': 'width: 70%; margin-right: 8px;',
				'readonly': 'readonly',
				'value': url
			});
			return E('div', { 'style': 'margin-bottom: 12px;' }, [
				E('div', { 'style': 'font-weight:600; margin-bottom:4px;' }, label),
				E('div', { 'style': 'display:flex; align-items:center;' }, [
					input,
					E('button', {
						'class': 'cbi-button cbi-button-action',
						'click': function(ev) {
							input.select();
							document.execCommand('copy');
							ui.addNotification(null, E('p', {}, '已复制订阅链接到剪贴板！'), 'info');
						}
					}, '📋 复制链接')
				])
			]);
		}

		var exportSection = E('div', { 'class': 'cbi-section' }, [
			E('h3', {}, _('下游透明代理插件联动 (优质活节点导出)')),
			E('p', { 'class': 'cbi-section-descr' }, _('在 OpenClash、PassWall、SSR-Plus 中直接填入以下订阅地址，即可享受 VPN Parser 全自动测速清洗后的纯净活节点：')),
			makeCopyRow('OpenClash 配置订阅地址 (Clash YAML 标准格式)', links.clash || ''),
			makeCopyRow('PassWall / SSR-Plus 节点订阅地址 (Base64 协议流)', links.uri || ''),
			makeCopyRow('完整 JSON 格式导出 (适合自建脚本调用)', links.json || '')
		]);
		viewRoot.appendChild(exportSection);

		return viewRoot;
	},

	handleSaveApply: null,
	handleSave: null,
	handleReset: null
});
