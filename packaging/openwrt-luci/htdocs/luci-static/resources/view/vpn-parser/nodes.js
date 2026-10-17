'use strict';
'require view';
'require ui';
'require rpc';

var callListNodes = rpc.declare({
	object: 'luci.vpn-parser',
	method: 'list_nodes',
	params: ['page', 'page_size', 'latency', 'protocol', 'keyword', 'sort']
});

var callSpeedtestOne = rpc.declare({
	object: 'luci.vpn-parser',
	method: 'speedtest_one',
	params: ['id']
});

var callSpeedtest = rpc.declare({
	object: 'luci.vpn-parser',
	method: 'speedtest',
	params: ['ids']
});

var callDeleteNode = rpc.declare({
	object: 'luci.vpn-parser',
	method: 'delete_node',
	params: ['id']
});

var callCleanNodes = rpc.declare({
	object: 'luci.vpn-parser',
	method: 'clean_nodes',
	params: ['action']
});

return view.extend({
	filterState: {
		page: 1,
		pageSize: 50,
		latency: 'alive', // 默认优先看可用节点
		protocol: '',
		keyword: '',
		sort: 'latency'
	},

	load: function() {
		return this.fetchNodes();
	},

	fetchNodes: function() {
		var s = this.filterState;
		var lat = (s.latency === 'all') ? '' : s.latency;
		return callListNodes(s.page, s.pageSize, lat, s.protocol, s.keyword, s.sort);
	},

	renderProtocolBadge: function(proto) {
		var p = (proto || '').toLowerCase();
		var bg = '#7f8c8d';
		if (p === 'vless') bg = '#2980b9';
		else if (p === 'vmess') bg = '#8e44ad';
		else if (p === 'trojan') bg = '#16a085';
		else if (p === 'ss' || p === 'shadowsocks') bg = '#d35400';
		else if (p === 'hysteria2' || p === 'hy2') bg = '#c0392b';
		else if (p === 'tuic') bg = '#27ae60';

		return E('span', {
			'style': 'background-color:' + bg + '; color:#fff; padding:2px 8px; border-radius:10px; font-size:11px; font-weight:bold; text-transform:uppercase;'
		}, p);
	},

	renderLatencyBadge: function(node) {
		if (node.latency_ok === true && typeof node.latency_ms === 'number') {
			var ms = node.latency_ms;
			var color = '#27ae60';
			if (ms > 300) color = '#e67e22';
			else if (ms > 150) color = '#f39c12';

			var text = ms + ' ms';
			if (node.download_kbps && node.download_kbps > 0) {
				text += ' (' + (node.download_kbps / 1024).toFixed(1) + ' MB/s)';
			}
			return E('span', {
				'style': 'color:' + color + '; font-weight:bold; font-size:12px;'
			}, '🟢 ' + text);
		} else if (node.latency_ok === false) {
			return E('span', {
				'style': 'color:#e74c3c; font-weight:bold; font-size:12px;'
			}, '🔴 超时/失效');
		} else {
			return E('span', {
				'style': 'color:#95a5a6; font-size:12px;'
			}, '⚪ 未测速');
		}
	},

	render: function(res) {
		var self = this;
		var data = (res && res.data) ? res.data : (res || {});
		var nodes = data.items || [];
		var total = data.total || 0;
		var totalPages = data.total_pages || 1;

		var viewRoot = E('div', { 'class': 'cbi-map' }, [
			E('h2', {}, _('VPN Parser - 节点池管理')),
			E('div', { 'class': 'cbi-map-descr' }, _('浏览与运维数据库中的所有代理节点，支持真握手测速、可用性过滤与一键导出'))
		]);

		// 1. 过滤检索栏
		var latencySelect = E('select', { 'class': 'cbi-input-select', 'style': 'margin-right:8px;' }, [
			E('option', { 'value': 'all', 'selected': self.filterState.latency === 'all' ? 'selected' : null }, _('全部状态')),
			E('option', { 'value': 'alive', 'selected': self.filterState.latency === 'alive' ? 'selected' : null }, _('🟢 仅看可用')),
			E('option', { 'value': 'dead', 'selected': self.filterState.latency === 'dead' ? 'selected' : null }, _('🔴 仅看失效')),
			E('option', { 'value': 'untested', 'selected': self.filterState.latency === 'untested' ? 'selected' : null }, _('⚪ 仅看未测速'))
		]);

		var protoSelect = E('select', { 'class': 'cbi-input-select', 'style': 'margin-right:8px;' }, [
			E('option', { 'value': '', 'selected': !self.filterState.protocol ? 'selected' : null }, _('全部协议')),
			E('option', { 'value': 'vless', 'selected': self.filterState.protocol === 'vless' ? 'selected' : null }, 'VLESS'),
			E('option', { 'value': 'vmess', 'selected': self.filterState.protocol === 'vmess' ? 'selected' : null }, 'VMess'),
			E('option', { 'value': 'trojan', 'selected': self.filterState.protocol === 'trojan' ? 'selected' : null }, 'Trojan'),
			E('option', { 'value': 'ss', 'selected': self.filterState.protocol === 'ss' ? 'selected' : null }, 'Shadowsocks'),
			E('option', { 'value': 'hysteria2', 'selected': self.filterState.protocol === 'hysteria2' ? 'selected' : null }, 'Hysteria2'),
			E('option', { 'value': 'tuic', 'selected': self.filterState.protocol === 'tuic' ? 'selected' : null }, 'TUIC')
		]);

		var keywordInput = E('input', {
			'type': 'text',
			'class': 'cbi-input-text',
			'placeholder': _('搜索节点名/域名/IP...'),
			'value': self.filterState.keyword,
			'style': 'width: 220px; margin-right: 8px;'
		});

		var applyFilterBtn = E('button', {
			'class': 'cbi-button cbi-button-action',
			'click': function() {
				self.filterState.latency = latencySelect.value;
				self.filterState.protocol = protoSelect.value;
				self.filterState.keyword = keywordInput.value.trim();
				self.filterState.page = 1;
				self.fetchNodes().then(function(newRes) {
					var newRoot = self.render(newRes);
					viewRoot.parentNode.replaceChild(newRoot, viewRoot);
				});
			}
		}, '🔍 筛选');

		var speedtestCurrentBtn = E('button', {
			'class': 'cbi-button cbi-button-apply',
			'style': 'margin-left: 12px;',
			'click': function() {
				var ids = nodes.map(function(n) { return n.id; });
				if (ids.length === 0) {
					ui.addNotification(null, E('p', {}, '当前页无节点可测速'), 'warning');
					return;
				}
				ui.addNotification(null, E('p', {}, '已开始测速当前页 ' + ids.length + ' 个节点...'), 'info');
				callSpeedtest(ids).then(function() {
					ui.addNotification(null, E('p', {}, '测速任务已在后台执行'), 'success');
				});
			}
		}, '⚡ 测速当前页');

		var cleanDeadBtn = E('button', {
			'class': 'cbi-button cbi-button-remove',
			'style': 'margin-left: 8px;',
			'click': function() {
				if (!confirm('确定清理所有超时失效死节点吗？')) return;
				callCleanNodes('delete_dead').then(function() {
					ui.addNotification(null, E('p', {}, '失效节点清理完成！'), 'success');
					self.fetchNodes().then(function(newRes) {
						var newRoot = self.render(newRes);
						viewRoot.parentNode.replaceChild(newRoot, viewRoot);
					});
				});
			}
		}, '🧹 清理失效');

		var filterSection = E('div', { 'class': 'cbi-section', 'style': 'display:flex; flex-wrap:wrap; align-items:center; gap:8px;' }, [
			latencySelect,
			protoSelect,
			keywordInput,
			applyFilterBtn,
			speedtestCurrentBtn,
			cleanDeadBtn
		]);
		viewRoot.appendChild(filterSection);

		// 2. 节点总数与分页统计
		var infoRow = E('div', { 'style': 'margin: 10px 0; font-size:13px; color:#555; display:flex; justify-content:space-between; align-items:center;' }, [
			E('div', {}, '当前显示第 ' + ((self.filterState.page - 1) * self.filterState.pageSize + 1) + ' - ' + Math.min(self.filterState.page * self.filterState.pageSize, total) + ' 条，共 ' + total + ' 个节点'),
			E('div', {}, '第 ' + self.filterState.page + ' / ' + totalPages + ' 页')
		]);
		viewRoot.appendChild(infoRow);

		// 3. 节点列表表格
		var table = E('table', { 'class': 'table cbi-section-table' });
		table.appendChild(E('tr', { 'class': 'tr cbi-section-table-titles' }, [
			E('th', { 'class': 'th', 'style': 'width:80px;' }, _('协议')),
			E('th', { 'class': 'th' }, _('节点名称 / 服务器')),
			E('th', { 'class': 'th', 'style': 'width:100px;' }, _('地区')),
			E('th', { 'class': 'th', 'style': 'width:180px;' }, _('延迟状态')),
			E('th', { 'class': 'th cbi-section-table-cell-last', 'style': 'width:180px;' }, _('操作'))
		]));

		if (nodes.length === 0) {
			table.appendChild(E('tr', { 'class': 'tr' }, [
				E('td', { 'class': 'td', 'colspan': 5, 'style': 'text-align:center; padding:24px; color:#999;' }, _('没有匹配的节点'))
			]));
		} else {
			nodes.forEach(function(node) {
				var tr = E('tr', { 'class': 'tr' });

				// 协议
				tr.appendChild(E('td', { 'class': 'td' }, [self.renderProtocolBadge(node.protocol)]));

				// 名称与服务器
				tr.appendChild(E('td', { 'class': 'td' }, [
					E('div', { 'style': 'font-weight:600; font-size:13px; color:#2c3e50; word-break:break-all;' }, node.name || '未命名'),
					E('div', { 'style': 'font-size:11px; color:#7f8c8d; font-family:monospace; margin-top:2px;' }, (node.server || '') + ':' + (node.port || ''))
				]));

				// 地区
				tr.appendChild(E('td', { 'class': 'td' }, [
					E('span', { 'style': 'font-size:12px;' }, node.country || node.country_code || '-')
				]));

				// 延迟
				tr.appendChild(E('td', { 'class': 'td' }, [self.renderLatencyBadge(node)]));

				// 操作列
				var testBtn = E('button', {
					'class': 'cbi-button cbi-button-action',
					'style': 'margin-right:6px; font-size:11px; padding:2px 8px;',
					'click': function(ev) {
						testBtn.disabled = true;
						testBtn.innerText = '⏳ 测速中';
						callSpeedtestOne(node.id).then(function(res) {
							ui.addNotification(null, E('p', {}, '节点 ' + node.name + ' 测速完成！'), 'success');
							self.fetchNodes().then(function(newRes) {
								var newRoot = self.render(newRes);
								viewRoot.parentNode.replaceChild(newRoot, viewRoot);
							});
						}).catch(function(err) {
							ui.addNotification(null, E('p', {}, '测速失败: ' + err), 'error');
							testBtn.disabled = false;
							testBtn.innerText = '⚡ 测速';
						});
					}
				}, '⚡ 测速');

				var copyBtn = E('button', {
					'class': 'cbi-button cbi-button-neutral',
					'style': 'margin-right:6px; font-size:11px; padding:2px 8px;',
					'click': function(ev) {
						if (node.raw) {
							var temp = document.createElement('textarea');
							temp.value = node.raw;
							document.body.appendChild(temp);
							temp.select();
							document.execCommand('copy');
							document.body.removeChild(temp);
							ui.addNotification(null, E('p', {}, '已复制节点分享链接！'), 'info');
						} else {
							ui.addNotification(null, E('p', {}, '无原始链接可供复制'), 'warning');
						}
					}
				}, '📋 复制');

				var delBtn = E('button', {
					'class': 'cbi-button cbi-button-remove',
					'style': 'font-size:11px; padding:2px 8px;',
					'click': function(ev) {
						if (!confirm('确定删除节点: ' + (node.name || node.id) + ' 吗？')) return;
						callDeleteNode(node.id).then(function() {
							ui.addNotification(null, E('p', {}, '已删除该节点'), 'success');
							self.fetchNodes().then(function(newRes) {
								var newRoot = self.render(newRes);
								viewRoot.parentNode.replaceChild(newRoot, viewRoot);
							});
						});
					}
				}, '🗑️');

				tr.appendChild(E('td', { 'class': 'td cbi-section-table-cell-last' }, [testBtn, copyBtn, delBtn]));
				table.appendChild(tr);
			});
		}

		viewRoot.appendChild(E('div', { 'class': 'cbi-section' }, [table]));

		// 4. 分页控制底栏
		var prevBtn = E('button', {
			'class': 'cbi-button cbi-button-neutral',
			'disabled': self.filterState.page <= 1 ? 'disabled' : null,
			'click': function() {
				if (self.filterState.page > 1) {
					self.filterState.page--;
					self.fetchNodes().then(function(newRes) {
						var newRoot = self.render(newRes);
						viewRoot.parentNode.replaceChild(newRoot, viewRoot);
					});
				}
			}
		}, '◀ 上一页');

		var nextBtn = E('button', {
			'class': 'cbi-button cbi-button-neutral',
			'disabled': self.filterState.page >= totalPages ? 'disabled' : null,
			'style': 'margin-left: 12px;',
			'click': function() {
				if (self.filterState.page < totalPages) {
					self.filterState.page++;
					self.fetchNodes().then(function(newRes) {
						var newRoot = self.render(newRes);
						viewRoot.parentNode.replaceChild(newRoot, viewRoot);
					});
				}
			}
		}, '下一页 ▶');

		var paginationRow = E('div', { 'style': 'display:flex; justify-content:center; align-items:center; margin:16px 0;' }, [
			prevBtn,
			E('span', { 'style': 'margin: 0 16px; font-weight:600;' }, self.filterState.page + ' / ' + totalPages),
			nextBtn
		]);
		viewRoot.appendChild(paginationRow);

		return viewRoot;
	},

	handleSaveApply: null,
	handleSave: null,
	handleReset: null
});
