'use strict';
'require view';
'require ui';
'require rpc';
'require uci';

var callGetSettings = rpc.declare({
	object: 'luci.vpn-parser',
	method: 'get_settings'
});

var callUpdateSettings = rpc.declare({
	object: 'luci.vpn-parser',
	method: 'update_settings'
});

return view.extend({
	load: function() {
		return Promise.all([
			uci.load('vpn-parser'),
			callGetSettings().catch(function() { return {}; })
		]);
	},

	render: function(data) {
		var backendSettings = (data[1] && data[1].data) ? data[1].data : (data[1] || {});

		// 1. 读取 UCI 配置项
		var uciEnabled = uci.get('vpn-parser', 'main', 'enabled') !== '0';
		var uciListen = uci.get('vpn-parser', 'main', 'listen') || '0.0.0.0:8080';
		var uciDbPath = uci.get('vpn-parser', 'main', 'db_path') || '/etc/vpn-parser/vpn-parser.db';
		var uciApiToken = uci.get('vpn-parser', 'main', 'api_token') || '';

		var viewRoot = E('div', { 'class': 'cbi-map' }, [
			E('h2', {}, _('VPN Parser - 参数配置')),
			E('div', { 'class': 'cbi-map-descr' }, _('配置 OpenWrt 系统服务守护参数与 VPN Parser 高性能并发测速引擎调优'))
		]);

		// === 模块 1: OpenWrt 系统服务配置 ===
		var serviceEnabledCheck = E('input', {
			'type': 'checkbox',
			'checked': uciEnabled ? 'checked' : null
		});

		var listenInput = E('input', {
			'type': 'text',
			'class': 'cbi-input-text',
			'style': 'width: 320px;',
			'value': uciListen
		});

		var dbPathInput = E('input', {
			'type': 'text',
			'class': 'cbi-input-text',
			'style': 'width: 320px;',
			'value': uciDbPath
		});

		var tokenInput = E('input', {
			'type': 'password',
			'class': 'cbi-input-text',
			'style': 'width: 320px;',
			'value': uciApiToken,
			'placeholder': _('留空表示无需认证 (仅限局域网可信环境)')
		});

		var serviceSection = E('div', { 'class': 'cbi-section' }, [
			E('h3', {}, _('系统服务守护设置 (UCI)')),
			E('div', { 'class': 'cbi-value' }, [
				E('label', { 'class': 'cbi-value-title' }, _('启用 VPN Parser 微服务')),
				E('div', { 'class': 'cbi-value-field' }, [
					E('label', {}, [serviceEnabledCheck, ' ' + _('系统开机自启并由 procd 进程守护')])
				])
			]),
			E('div', { 'class': 'cbi-value' }, [
				E('label', { 'class': 'cbi-value-title' }, _('HTTP API 监听地址与端口')),
				E('div', { 'class': 'cbi-value-field' }, [
					listenInput,
					E('div', { 'class': 'cbi-value-description' }, _('默认 0.0.0.0:8080，如需仅限本机访问可设为 127.0.0.1:8080'))
				])
			]),
			E('div', { 'class': 'cbi-value' }, [
				E('label', { 'class': 'cbi-value-title' }, _('SQLite 数据库存储路径')),
				E('div', { 'class': 'cbi-value-field' }, [
					dbPathInput,
					E('div', { 'class': 'cbi-value-description' }, _('持久化节点与订阅库文件路径，请确保存放目录可写'))
				])
			]),
			E('div', { 'class': 'cbi-value' }, [
				E('label', { 'class': 'cbi-value-title' }, _('API Token 鉴权密钥')),
				E('div', { 'class': 'cbi-value-field' }, [
					tokenInput,
					E('div', { 'class': 'cbi-value-description' }, _('可选配置。配置后所有外部调用需携带 Authorization: Bearer <Token>'))
				])
			])
		]);
		viewRoot.appendChild(serviceSection);

		// === 模块 2: 高性能测速与微服务调优 ===
		var concurrencyInput = E('input', {
			'type': 'number',
			'class': 'cbi-input-text',
			'style': 'width: 120px;',
			'value': backendSettings.speedtest_concurrency || 32,
			'min': 1,
			'max': 128
		});

		var timeoutInput = E('input', {
			'type': 'number',
			'class': 'cbi-input-text',
			'style': 'width: 120px;',
			'value': backendSettings.speedtest_timeout_ms || 5000,
			'min': 500,
			'max': 30000
		});

		var tcpPingOnlyCheck = E('input', {
			'type': 'checkbox',
			'checked': backendSettings.speedtest_tcp_ping_only ? 'checked' : null
		});

		var dedupEgressCheck = E('input', {
			'type': 'checkbox',
			'checked': backendSettings.speedtest_dedup_egress ? 'checked' : null
		});

		var autoFetchCheck = E('input', {
			'type': 'checkbox',
			'checked': backendSettings.auto_fetch_enabled ? 'checked' : null
		});

		var fetchIntervalInput = E('input', {
			'type': 'number',
			'class': 'cbi-input-text',
			'style': 'width: 120px;',
			'value': backendSettings.auto_fetch_interval_min || 720,
			'min': 10
		});

		var autoSpeedtestCheck = E('input', {
			'type': 'checkbox',
			'checked': backendSettings.auto_speedtest_enabled ? 'checked' : null
		});

		var excludeKeywordsInput = E('input', {
			'type': 'text',
			'class': 'cbi-input-text',
			'style': 'width: 320px;',
			'value': backendSettings.exclude_keywords || '',
			'placeholder': _('例如: 剩余流量,过期时间,官网')
		});

		var engineSection = E('div', { 'class': 'cbi-section' }, [
			E('h3', {}, _('测速引擎与定时运维调优')),
			E('div', { 'class': 'cbi-value' }, [
				E('label', { 'class': 'cbi-value-title' }, _('测速并发数')),
				E('div', { 'class': 'cbi-value-field' }, [
					concurrencyInput,
					E('div', { 'class': 'cbi-value-description' }, _('同时进行真握手测速的节点数。软路由建议 32~64，小内存硬路由建议 8~16'))
				])
			]),
			E('div', { 'class': 'cbi-value' }, [
				E('label', { 'class': 'cbi-value-title' }, _('握手超时时间 (ms)')),
				E('div', { 'class': 'cbi-value-field' }, [
					timeoutInput,
					E('div', { 'class': 'cbi-value-description' }, _('超过该延迟判定为超时节点，默认 5000 毫秒'))
				])
			]),
			E('div', { 'class': 'cbi-value' }, [
				E('label', { 'class': 'cbi-value-title' }, _('纯 TCP Ping 节能探活模式')),
				E('div', { 'class': 'cbi-value-field' }, [
					E('label', {}, [tcpPingOnlyCheck, ' ' + _('开启后仅进行轻量 TCP 握手探活，极大降低路由器 CPU 与内存占用')]),
					E('div', { 'class': 'cbi-value-description' }, _('适合性能有限的硬路由设备，耗电极低且测速速度提升 5 倍以上'))
				])
			]),
			E('div', { 'class': 'cbi-value' }, [
				E('label', { 'class': 'cbi-value-title' }, _('同落地 IP 节点去重')),
				E('div', { 'class': 'cbi-value-field' }, [
					E('label', {}, [dedupEgressCheck, ' ' + _('同一出口 IP 只保留延迟最低的单个节点')])
				])
			]),
			E('div', { 'class': 'cbi-value' }, [
				E('label', { 'class': 'cbi-value-title' }, _('自动定时拉取订阅')),
				E('div', { 'class': 'cbi-value-field' }, [
					E('label', {}, [autoFetchCheck, ' ' + _('启用后台周期性静默拉取订阅')]),
					E('div', { 'style': 'margin-top:6px;' }, [
						_('间隔时间 (分钟): '),
						fetchIntervalInput
					])
				])
			]),
			E('div', { 'class': 'cbi-value' }, [
				E('label', { 'class': 'cbi-value-title' }, _('自动定时循环测速')),
				E('div', { 'class': 'cbi-value-field' }, [
					E('label', {}, [autoSpeedtestCheck, ' ' + _('在拉取新节点后自动发起全量真握手测速')])
				])
			]),
			E('div', { 'class': 'cbi-value' }, [
				E('label', { 'class': 'cbi-value-title' }, _('节点名排除关键词')),
				E('div', { 'class': 'cbi-value-field' }, [
					excludeKeywordsInput,
					E('div', { 'class': 'cbi-value-description' }, _('包含此类关键词的节点将在采集入库时直接丢弃，多个关键词用逗号分隔'))
				])
			])
		]);
		viewRoot.appendChild(engineSection);

		// === 模块 3: 保存并应用按钮 ===
		var saveBtn = E('button', {
			'class': 'cbi-button cbi-button-apply',
			'click': function() {
				ui.showModal(_('正在保存设置'), [
					E('p', { 'class': 'spinning' }, _('正在将配置写入 UCI 系统文件并同步微服务引擎...'))
				]);

				// 1. 保存 UCI
				uci.set('vpn-parser', 'main', 'enabled', serviceEnabledCheck.checked ? '1' : '0');
				uci.set('vpn-parser', 'main', 'listen', listenInput.value.trim());
				uci.set('vpn-parser', 'main', 'db_path', dbPathInput.value.trim());
				uci.set('vpn-parser', 'main', 'api_token', tokenInput.value.trim());

				var uciPromise = uci.save().then(function() {
					return uci.apply();
				});

				// 2. 保存 Backend Settings
				var newSettings = Object.assign({}, backendSettings, {
					speedtest_concurrency: parseInt(concurrencyInput.value) || 32,
					speedtest_timeout_ms: parseInt(timeoutInput.value) || 5000,
					speedtest_tcp_ping_only: tcpPingOnlyCheck.checked,
					speedtest_dedup_egress: dedupEgressCheck.checked,
					auto_fetch_enabled: autoFetchCheck.checked,
					auto_fetch_interval_min: parseInt(fetchIntervalInput.value) || 720,
					auto_speedtest_enabled: autoSpeedtestCheck.checked,
					exclude_keywords: excludeKeywordsInput.value.trim()
				});

				var apiPromise = callUpdateSettings(newSettings);

				Promise.all([uciPromise, apiPromise]).then(function() {
					ui.hideModal();
					ui.addNotification(null, E('p', {}, '配置已成功保存并应用！'), 'success');
					window.location.reload();
				}).catch(function(err) {
					ui.hideModal();
					ui.addNotification(null, E('p', {}, '配置保存失败: ' + err), 'error');
				});
			}
		}, _('💾 保存并应用'));

		var footerActions = E('div', { 'class': 'cbi-page-actions' }, [
			saveBtn,
			E('button', {
				'class': 'cbi-button cbi-button-reset',
				'style': 'margin-left:12px;',
				'click': function() { window.location.reload(); }
			}, _('重置'))
		]);
		viewRoot.appendChild(footerActions);

		return viewRoot;
	},

	handleSaveApply: null,
	handleSave: null,
	handleReset: null
});
