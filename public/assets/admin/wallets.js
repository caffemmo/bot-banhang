    function formatVnd(amount) {
      return Number(amount || 0).toLocaleString('vi-VN') + ' ₫';
    }

    function walletUserLabel(user) {
      const name = user.full_name || [user.first_name, user.last_name].filter(Boolean).join(' ');
      if (name) return name;
      if (user.username) return `@${user.username}`;
      return `User ${user.user_id}`;
    }

    function walletTxTypeLabel(type) {
      const map = {
        topup: 'Nạp tiền',
        purchase: 'Thanh toán',
        refund: 'Hoàn tiền',
        admin_adjust: 'Điều chỉnh',
      };
      return map[type] || type || '-';
    }

    function applyWalletFilters() {
      walletOffset = 0;
      loadWallets();
    }

    async function loadWallets() {
      const q = $('#wallet-query').val().trim();
      const query = q ? `&query=${encodeURIComponent(q)}` : '';
      setFilterLoading('#wallets-filter-loading', true);
      try {
        const data = await apiFetch(`/wallets?limit=20&offset=${walletOffset}${query}`);
        const pageBalance = data.items.reduce((sum, item) => sum + Number(item.balance || 0), 0);
        $('#wallet-total-users').text(Number(data.total || 0).toLocaleString('vi-VN'));
        $('#wallet-page-balance').text(formatVnd(pageBalance));
        $('#wallets-meta').text(`total=${data.total} · offset=${data.offset} · limit=${data.limit}`);

        const rows = data.items.map(user => {
          const label = walletUserLabel(user);
          const usernameLine = user.username ? `<div class="small text-muted">@${escapeHtml(user.username)}</div>` : '';
          const nameLine = label === `@${user.username}` ? '' : `<div class="fw-semibold">${escapeHtml(label)}</div>`;
          const chat = user.chat_id ? `<code>${escapeHtml(user.chat_id)}</code>` : '<span class="text-muted">-</span>';
          const updated = user.wallet_updated_at || user.last_transaction_at || user.user_updated_at || user.user_created_at || '-';
          return `
          <tr>
            <td class="mobile-priority">
              ${nameLine || `<div class="fw-semibold">${escapeHtml(label)}</div>`}
              ${usernameLine}
              <small class="text-muted">ID: <code>${escapeHtml(user.user_id)}</code></small>
            </td>
            <td class="d-none d-md-table-cell mobile-hide">${chat}</td>
            <td class="mobile-priority"><span class="fw-bold text-primary">${formatVnd(user.balance)}</span></td>
            <td class="d-none d-md-table-cell mobile-hide">${Number(user.transaction_count || 0).toLocaleString('vi-VN')}</td>
            <td class="d-none d-lg-table-cell mobile-hide small text-muted">${escapeHtml(updated)}</td>
            <td class="mobile-priority">
              <div class="dropdown">
                <button class="btn btn-sm btn-light border dropdown-toggle touch-target" type="button" data-bs-toggle="dropdown" aria-label="Tùy chọn ví">Ví</button>
                <ul class="dropdown-menu shadow-sm border-0">
                  <li><a class="dropdown-item view-wallet" href="#" data-user-id="${user.user_id}" data-user-label="${escapeAttr(label)}">Chi tiết</a></li>
                  <li><a class="dropdown-item topup-wallet" href="#" data-user-id="${user.user_id}" data-user-label="${escapeAttr(label)}">Nạp tiền</a></li>
                  <li><a class="dropdown-item adjust-wallet" href="#" data-user-id="${user.user_id}" data-user-label="${escapeAttr(label)}">Điều chỉnh +/-</a></li>
                </ul>
              </div>
            </td>
          </tr>
        `;
        }).join('');
        $('#wallets-body').html(rows || `<tr><td colspan="6" class="text-center text-muted">${I18N.common.empty}</td></tr>`);
      } catch (e) {
        alertBox('danger', `${I18N.alert.loadWalletsFailed}: ${e.message}`);
      } finally {
        setFilterLoading('#wallets-filter-loading', false);
      }
    }

    async function viewWallet(userId, label = '') {
      try {
        const data = await apiFetch(`/wallets/${userId}`);
        currentWalletUserId = Number(userId);
        const wallet = data.wallet || {};
        const transactions = data.transactions || [];
        $('#wallet-detail-user').text(`${label || 'User'} · ID ${userId}`);
        $('#wallet-detail-balance').text(formatVnd(wallet.balance));
        $('#wallet-detail-updated').text(wallet.updated_at || '-');
        $('#wallet-detail-topup').data('user-id', userId).data('user-label', label || `User ${userId}`);
        const txRows = transactions.map(tx => {
          const amount = Number(tx.amount || 0);
          const amountClass = amount >= 0 ? 'text-success' : 'text-danger';
          return `
          <tr>
            <td><small>${escapeHtml(tx.created_at || '-')}</small></td>
            <td>${escapeHtml(walletTxTypeLabel(tx.tx_type))}</td>
            <td><span class="fw-semibold ${amountClass}">${formatVnd(amount)}</span></td>
            <td>${formatVnd(tx.balance_after)}</td>
            <td class="small text-break">${escapeHtml(tx.note || tx.order_id || tx.topup_id || '-')}</td>
          </tr>
        `;
        }).join('');
        $('#wallet-transactions-body').html(txRows || '<tr><td colspan="5" class="text-center text-muted">Chưa có giao dịch</td></tr>');
        new bootstrap.Modal(document.getElementById('walletModal')).show();
      } catch (e) {
        alertBox('danger', 'Tải chi tiết ví thất bại: ' + e.message);
      }
    }

    function openManualTopup(userId = null, label = '') {
      const fields = [];
      if (!userId) {
        fields.push({ name: 'user_id', label: 'User ID', type: 'number', required: true, placeholder: '123456789' });
      }
      fields.push(
        { name: 'amount', label: 'Số tiền nạp', type: 'number', required: true, placeholder: '100000' },
        { name: 'note', label: 'Ghi chú', type: 'text', required: false, placeholder: 'Nạp thủ công' },
        { name: 'setup_code', label: 'Mã bí mật', type: 'password', required: true }
      );
      showInputActionModal({
        title: 'Nạp tiền thủ công',
        description: userId ? `${label || 'User'} · ID ${userId}` : 'Nhập User ID cần nạp.',
        confirmText: 'Nạp tiền',
        fields,
        onSubmit: async (values) => {
          const targetUserId = Number(userId || values.user_id);
          const amount = Number(values.amount);
          if (!Number.isInteger(targetUserId) || targetUserId <= 0) {
            throw new Error('User ID không hợp lệ.');
          }
          if (!Number.isInteger(amount) || amount <= 0) {
            throw new Error('Số tiền nạp phải lớn hơn 0.');
          }
          await apiFetch(`/wallets/${targetUserId}/topup`, {
            method: 'POST',
            body: JSON.stringify({
              amount,
              note: values.note || null,
              setup_code: values.setup_code,
            }),
          });
          alertBox('success', I18N.alert.walletTopupSuccess);
          await loadWallets();
          if (currentWalletUserId === targetUserId) await viewWallet(targetUserId, label || `User ${targetUserId}`);
        },
      });
    }

    function openWalletAdjust(userId, label = '') {
      showInputActionModal({
        title: 'Điều chỉnh ví',
        description: `${label || 'User'} · ID ${userId}`,
        confirmText: 'Điều chỉnh',
        fields: [
          { name: 'amount', label: 'Số tiền (+ cộng, - trừ)', type: 'number', required: true, placeholder: '50000 hoặc -50000' },
          { name: 'note', label: 'Ghi chú', type: 'text', required: false, placeholder: 'Lý do điều chỉnh' },
          { name: 'setup_code', label: 'Mã bí mật', type: 'password', required: true },
        ],
        onSubmit: async (values) => {
          const amount = Number(values.amount);
          if (!Number.isInteger(amount) || amount === 0) {
            throw new Error('Số tiền điều chỉnh không hợp lệ.');
          }
          await apiFetch(`/wallets/${userId}/adjust`, {
            method: 'POST',
            body: JSON.stringify({
              amount,
              note: values.note || null,
              setup_code: values.setup_code,
            }),
          });
          alertBox('success', I18N.alert.walletAdjustSuccess);
          await loadWallets();
          if (currentWalletUserId === Number(userId)) await viewWallet(userId, label || `User ${userId}`);
        },
      });
    }

