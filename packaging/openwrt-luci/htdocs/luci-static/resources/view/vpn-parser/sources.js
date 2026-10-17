'use strict';
'require view';
'require ui';
'require rpc';

var callListSources = rpc.declare({
	object: 'luci.vpn-parser',
	method: 'list_sources'
});

var callCreateSource = rpc.declare({
	object: 'luci.vpn-parser',
	method: 'create_source',
	params: ['name', 'url_template', 'enabled', 'remark']
});

var callUpdateSource = rpc.declare({
	object: 'luci.vpn-parser',
	method: 'update_source',
	params: ['id', 'name', 'url_template', 'enabled', 'remark']
});

var callDeleteSource = rpc.declare({
	object: 'luci.vpn-parser',
	method: 'delete_source',
	params: ['id']
});

var callFetchSource = rpc.declare({
	object: 'luci.vpn-parser',
	method: 'fetch_source',
	params: ['id']
});

var callFetchAll = rpc.declare({
	object: 'luci.vpn-parser',
	method: 'fetch_all'
});

var callDiscoverGists = rpc.declare({
	object: 'luci.vpn-parser',
	method: 'discover_gists',
	params: ['query', 'max_pages']
});

var callSpeedtest = rpc.declare({
	object: 'luci.vpn-parser',
	method: 'speedtest',
	params: ['ids']
});

return view.extend({
	load: function() {
		return callListSources();
	},

	showSourceModal: function(source, onSaved) {
		var isEdit = !!source;
		var title = isEdit ? _('编辑订阅源') : _('添加新订阅源');

		var nameInput = E('input', {
			'type': 'text',
			'class': 'cbi-input-text',
			'style': 'width:100%;',
			'value': isEdit ? source.name : '',
			'placeholder': _('例如: 每日公开源 A')
		});

		var urlInput = E('input', {
			'type': 'text',
			'class': 'cbi-input-text',
			'style': 'width:100%;',
			'value': isEdit ? source.url_template : '',
			'placeholder': _('支持占位符: https://example.com/sub_{Ymd}.txt')
		});

		var enabledCheck = E('input', {
			'type': 'checkbox',
			'checked': isEdit ? (source.enabled ? 'checked' : null) : 'checked'
		});

		var remarkInput = E('input', {
			'type': 'text',
			'class': 'cbi-input-text',
			'style': 'width:100%;',
			'value': isEdit ? (source.remark || '') : '',
			'placeholder': _('可选备注信息')
		});

		var modalContent = E('div', { 'class': 'cbi-map' }, [
			E('div', { 'class': 'cbi-value' }, [
				E('label', { 'class': 'cbi-value-title' }, _('订阅名称')),
				E('div', { 'class': 'cbi-value-field' }, [nameInput])
			]),
			E('div', { 'class': 'cbi-value' }, [
				E('label', { 'class': 'cbi-value-title' }, _('订阅地址 (URL)')),
				E('div', { 'class': 'cbi-value-field' }, [
					urlInput,
					E('div', { 'class': 'cbi-value-description' }, _('支持日期动态模板占位符，例如: {Ymd}, {Y}, {m}, {d}'))
				])
			]),
			E('div', { 'class': 'cbi-value' }, [
				E('label', { 'class': 'cbi-value-title' }, _('是否启用')),
				E('div', { 'class': 'cbi-value-field' }, [
					E('label', {}, [enabledCheck, ' ' + _('拉取与同步')])
				])
			]),
			E('div', { 'class': 'cbi-value' }, [
				E('label', { 'class': 'cbi-value-title' }, _('备注')),
				E('div', { 'class': 'cbi-value-field' }, [remarkInput])
			])
		]);

		ui.showModal(title, [
			modalContent,
			E('div', { 'class': 'cbi-page-actions' }, [
				E('button', {
					'class': 'cbi-button cbi-button-apply',
					'click': function() {
						var name = nameInput.value.trim();
						var url = urlInput.value.trim();
						if (!name || !url) {
							ui.addNotification(null, E('p', {}, '名称和订阅地址不能为空'), 'warning');
							return;
						}
						var enabled = enabledCheck.checked;
						var remark = remarkInput.value.trim();

						var promise = isEdit
							? callUpdateSource(source.id, name, url, enabled, remark)
							: callCreateSource(name, url, enabled, remark);

						promise.then(function() {
							ui.hideModal();
							ui.addNotification(null, E('p', {}, isEdit ? '订阅源已更新' : '订阅源已添加'), 'success');
							if (onSaved) onSaved();
						}).catch(function(err) {
							ui.addNotification(null, E('p', {}, '保存失败: ' + err), 'error');
						});
					}
				}, _('保存')),
				E('button', {
					'class': 'cbi-button cbi-button-reset',
					'click': function() { ui.hideModal(); }
				}, _('取消'))
			])
		]);
	},

	render: function(res) {
		var self = this;
		var sources = (res && res.data) ? res.data : (Array.isArray(res) ? res : []);

		var viewRoot = E('div', { 'class': 'cbi-map' }, [
			E('h2', {}, _('VPN Parser - 订阅源管理')),
			E('div', { 'class': 'cbi-map-descr' }, _('管理所有订阅拉取源，支持动态日期占位符、HTTP 条件缓存 (ETag / Last-Modified) 与并发采集'))
		]);

		function reloadView() {
			self.load().then(function(newRes) {
				var newRoot = self.render(newRes);
				viewRoot.parentNode.replaceChild(newRoot, viewRoot);
			});
		}

		// 1. 全局操作栏
		var actionSection = E('div', { 'class': 'cbi-section', 'style': 'display:flex; flex-wrap:wrap; gap:12px; align-items:center;' }, [
			E('button', {
				'class': 'cbi-button cbi-button-apply',
				'click': function() {
					self.showSourceModal(null, reloadView);
				}
			}, '➕ 添加新订阅源'),
			E('button', {
				'class': 'cbi-button cbi-button-action',
				'click': function() {
					ui.addNotification(null, E('p', {}, '已触发全量拉取已启用的订阅源...'), 'info');
					callFetchAll().then(function() {
						ui.addNotification(null, E('p', {}, '全量订阅拉取完成！'), 'success');
						reloadView();
					});
				}
			}, '🔄 立即拉取全部'),
			E('button', {
				'class': 'cbi-button cbi-button-neutral',
				'click': function() {
					ui.addNotification(null, E('p', {}, '开始全量节点连通测速...'), 'info');
					callSpeedtest([]).then(function() {
						ui.addNotification(null, E('p', {}, '全量测速已在后台启动！'), 'success');
					});
				}
			}, '⚡ 全量节点测速')
		]);
		viewRoot.appendChild(actionSection);

		// 2. 订阅源列表表格
		var table = E('table', { 'class': 'table cbi-section-table' });
		table.appendChild(E('tr', { 'class': 'tr cbi-section-table-titles' }, [
			E('th', { 'class': 'th', 'style': 'width:60px;' }, _('状态')),
			E('th', { 'class': 'th', 'style': 'width:180px;' }, _('订阅名称')),
			E('th', { 'class': 'th' }, _('订阅 URL 模板')),
			E('th', { 'class': 'th', 'style': 'width:90px; text-align:center;' }, _('节点数')),
			E('th', { 'class': 'th', 'style': 'width:160px;' }, _('最近拉取')),
			E('th', { 'class': 'th cbi-section-table-cell-last', 'style': 'width:200px;' }, _('操作'))
		]));

		if (sources.length === 0) {
			table.appendChild(E('tr', { 'class': 'tr' }, [
				E('td', { 'class': 'td', 'colspan': 6, 'style': 'text-align:center; padding:24px; color:#999;' }, _('暂无订阅源，请点击上方按钮添加或使用 Gist 发现'))
			]));
		} else {
			sources.forEach(function(src) {
				var tr = E('tr', { 'class': 'tr' });

				// 启用徽章
				var statusBadge = src.enabled
					? E('span', { 'style': 'color:#27ae60; font-weight:bold; font-size:12px;' }, '🟢 启用')
					: E('span', { 'color': '#7f8c8d', 'style': 'color:#95a5a6; font-size:12px;' }, '⚪ 停用');
				tr.appendChild(E('td', { 'class': 'td' }, [statusBadge]));

				// 名称
				tr.appendChild(E('td', { 'class': 'td' }, [
					E('div', { 'style': 'font-weight:600; font-size:13px; color:#2c3e50;' }, src.name),
					src.remark ? E('div', { 'style': 'font-size:11px; color:#888;' }, src.remark) : ''
				]));

				// URL 模板
				tr.appendChild(E('td', { 'class': 'td' }, [
					E('div', {
						'style': 'max-width:340px; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; font-family:monospace; font-size:11px; color:#555;',
						'title': src.url_template
					}, src.url_template)
				]));

				// 节点数
				tr.appendChild(E('td', { 'class': 'td', 'style': 'text-align:center;' }, [
					E('span', {
						'style': 'background:#eef2f7; color:#2c3e50; padding:2px 8px; border-radius:10px; font-weight:bold; font-size:12px;'
					}, (src.last_node_count || 0))
				]));

				// 最近拉取状态与时间
				var statusStr = src.last_status || '从未拉取';
				var isSuccess = statusStr.indexOf('成功') !== -1 || statusStr.indexOf('ok') !== -1 || statusStr.indexOf('200') !== -1;
				var statusColor = isSuccess ? '#27ae60' : (statusStr === '从未拉取' ? '#95a5a6' : '#e74c3c');

				var timeStr = src.last_fetch_at ? src.last_fetch_at.replace('T', ' ').substring(0, 19) : '-';

				tr.appendChild(E('td', { 'class': 'td' }, [
					E('div', { 'style': 'font-size:11px; color:' + statusColor + '; font-weight:bold;' }, statusStr),
					E('div', { 'style': 'font-size:11px; color:#888;' }, timeStr)
				]));

				// 操作列
				var fetchBtn = E('button', {
					'class': 'cbi-button cbi-button-action',
					'style': 'margin-right:6px; font-size:11px; padding:2px 8px;',
					'click': function() {
						fetchBtn.disabled = true;
						fetchBtn.innerText = '⏳ 拉取中';
						callFetchSource(src.id).then(function() {
							ui.addNotification(null, E('p', {}, '订阅源 ' + src.name + ' 拉取完成！'), 'success');
							reloadView();
						}).catch(function(err) {
							ui.addNotification(null, E('p', {}, '拉取失败: ' + err), 'error');
							fetchBtn.disabled = false;
							fetchBtn.innerText = '🔄 拉取';
						});
					}
				}, '🔄 拉取');

				var editBtn = E('button', {
					'class': 'cbi-button cbi-button-neutral',
					'style': 'margin-right:6px; font-size:11px; padding:2px 8px;',
					'click': function() {
						self.showSourceModal(src, reloadView);
					}
				}, '✏️ 编辑');

				var delBtn = E('button', {
					'class': 'cbi-button cbi-button-remove',
					'style': 'font-size:11px; padding:2px 8px;',
					'click': function() {
						if (!confirm('确定删除订阅源: ' + src.name + ' 吗？')) return;
						callDeleteSource(src.id).then(function() {
							ui.addNotification(null, E('p', {}, '已删除订阅源'), 'success');
							reloadView();
						});
					}
				}, '🗑️');

				tr.appendChild(E('td', { 'class': 'td cbi-section-table-cell-last' }, [fetchBtn, editBtn, delBtn]));
				table.appendChild(tr);
			});
		}

		viewRoot.appendChild(E('div', { 'class': 'cbi-section' }, [table]));

		// 3. GitHub Gist 发现集成卡片
		var gistQueryInput = E('input', {
			'type': 'text',
			'class': 'cbi-input-text',
			'style': 'width: 200px; margin-right: 8px;',
			'value': 'vless//'
		});

		var gistPagesInput = E('input', {
			'type': 'number',
			'class': 'cbi-input-text',
			'style': 'width: 80px; margin-right: 8px;',
			'value': 20,
			'min': 1,
			'max': 100
		});

		var gistDiscoverBtn = E('button', {
			'class': 'cbi-button cbi-button-apply',
			'click': function() {
				var query = gistQueryInput.value.trim() || 'vless//';
				var maxPages = parseInt(gistPagesInput.value) || 20;

				ui.addNotification(null, E('p', {}, '已在后台启动 GitHub Gist 公开节点发现 (关键词: ' + query + ')...'), 'info');
				callDiscoverGists(query, maxPages).then(function() {
					ui.addNotification(null, E('p', {}, 'Gist 发现任务已启动，请稍后刷新查看新增源！'), 'success');
				}).catch(function(err) {
					ui.addNotification(null, E('p', {}, '启动发现任务失败: ' + err), 'error');
				});
			}
		}, '🚀 启动 Gist 自动发现');

		var gistSection = E('div', { 'class': 'cbi-section' }, [
			E('h3', {}, _('GitHub Gist 公开节点自动搜集发现')),
			E('p', { 'class': 'cbi-section-descr' }, _('无需手工收集订阅！通过 GitHub 全网搜索指定协议特征，自动挖掘包含真实可用节点的最新 Gist 订阅源并入库：')),
			E('div', { 'style': 'display:flex; align-items:center; gap:8px; margin-top:8px;' }, [
				E('span', { 'style': 'font-size:13px;' }, _('搜索关键词:')),
				gistQueryInput,
				E('span', { 'style': 'font-size:13px;' }, _('扫描页数:')),
				gistPagesInput,
				gistDiscoverBtn
			])
		]);
		viewRoot.appendChild(gistSection);

		return viewRoot;
	},

	handleSaveApply: null,
	handleSave: null,
	handleReset: null
});
