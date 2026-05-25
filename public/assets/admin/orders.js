    async function loadOrders() {
      const statusVal = $('#order-status').val();
      const status = statusVal === 'all' ? '' : `&status=${statusVal}`;
      const q = $('#order-query').val().trim();
      const query = q ? `&query=${encodeURIComponent(q)}` : '';
      const from = $('#order-from').val().trim();
      const to = $('#order-to').val().trim();
      const fromQ = from ? `&from=${encodeURIComponent(from)}` : '';
      const toQ = to ? `&to=${encodeURIComponent(to)}` : '';

      setFilterLoading('#orders-filter-loading', true);
      try {
        const data = await apiFetch(`/orders?limit=20&offset=${orderOffset}${status}${query}${fromQ}${toQ}`);
        const rows = data.items.map(row => {
          const o = row.order, p = row.product;
          const orderId = String(o.id || '');
          const orderShort = orderId.substring(0, 8);
          const reservationMode = o.reservation_mode === 'no_reserve'
            ? '<div><span class="badge bg-warning text-dark mt-1">no-reserve</span></div>'
            : '';
          return `<tr>
          <td class="mobile-priority"><div class="small fw-bold">${escapeHtml(o.created_at)}</div></td>
          <td class="d-none d-md-table-cell mobile-hide"><small>${escapeHtml(o.bank_memo)}</small></td>
          <td class="mobile-priority"><code>${escapeHtml(orderShort)}</code></td>
          <td class="mobile-priority"><span class="fw-bold">${escapeHtml(p.name)}</span></td>
          <td class="d-none d-md-table-cell mobile-hide">${o.qty}</td>
          <td class="mobile-priority"><span class="text-primary fw-bold">${o.amount.toLocaleString('vi-VN')}</span></td>
          <td class="mobile-priority">${badgeStatus(o.status)}${reservationMode}</td>
          <td class="d-none d-lg-table-cell mobile-hide small">${escapeHtml(o.user_id)}</td>
          <td class="d-none d-md-table-cell mobile-hide">${o.customer_input ? `<span class="text-break small">${escapeHtml(o.customer_input)}</span>` : '<span class="text-muted">-</span>'}</td>
          <td class="mobile-priority">
            <div class="dropdown">
              <button class="btn btn-sm btn-light border dropdown-toggle touch-target" type="button" data-bs-toggle="dropdown" aria-label="Tùy chọn xử lý đơn hàng">Xử lý</button>
              <ul class="dropdown-menu shadow-sm border-0">
                <li><a class="dropdown-item view-order" href="#" data-id="${escapeAttr(orderId)}">Chi tiết</a></li>
                <li><a class="dropdown-item mark-paid" href="#" data-id="${escapeAttr(orderId)}">Xác nhận thanh toán</a></li>
                <li><a class="dropdown-item resend" href="#" data-id="${escapeAttr(orderId)}">Gửi lại dữ liệu</a></li>
                <li><hr class="dropdown-divider"></li>
                <li><a class="dropdown-item cancel text-danger" href="#" data-id="${escapeAttr(orderId)}">Hủy đơn</a></li>
              </ul>
            </div>
          </td>
        </tr>`;
        }).join('');
        $('#orders-body').html(rows || `<tr><td colspan="10" class="text-center text-muted">${I18N.common.empty}</td></tr>`);
      } catch (e) {
        alertBox('danger', `${I18N.alert.loadOrdersFailed}: ${e.message}`);
      } finally {
        setFilterLoading('#orders-filter-loading', false);
      }
    }


    async function exportOrders() {
      const statusVal = $('#order-status').val();
      const status = statusVal === 'all' ? '' : `&status=${statusVal}`;
      const q = $('#order-query').val().trim();
      const query = q ? `&query=${encodeURIComponent(q)}` : '';
      const from = $('#order-from').val().trim();
      const to = $('#order-to').val().trim();
      const fromQ = from ? `&from=${encodeURIComponent(from)}` : '';
      const toQ = to ? `&to=${encodeURIComponent(to)}` : '';
      try {
        const res = await fetch(`/api/admin/orders/export?limit=10000${status}${query}${fromQ}${toQ}`, {
          credentials: 'same-origin',
        });
        if (!res.ok) throw new Error(res.statusText);
        const blob = await res.blob();
        const url = window.URL.createObjectURL(blob);
        const a = document.createElement('a');
        a.href = url;
        a.download = 'orders.csv';
        a.click();
        window.URL.revokeObjectURL(url);
      } catch (e) {
        alertBox('danger', `${I18N.alert.exportFailed}: ${e.message}`);
      }
    }


    async function viewOrder(id) {
      try {
        const data = await apiFetch(`/orders/${id}`);
        const order = data.order || {};
        const product = data.product || {};
        const status = order.status || '-';
        const delivered = order.delivered_data || '';
        const bankMemo = order.bank_memo || '';
        const parsedCreatedAt = order.created_at ? new Date(order.created_at) : null;
        const createdAt = parsedCreatedAt && !Number.isNaN(parsedCreatedAt.getTime())
          ? parsedCreatedAt.toLocaleString('vi-VN')
          : (order.created_at || '-');

        $('#order-detail-id').text(order.id || '-');
        $('#order-detail-status').html(badgeStatus(status));
        $('#order-detail-product').text(product.name || order.product_id || '-');
        $('#order-detail-qty').text(order.qty ?? '-');
        $('#order-detail-user').text(order.user_id || '-');
        $('#order-detail-created').text(createdAt);
        $('#order-detail-memo').text(bankMemo || '-');
        $('#order-detail-delivered').text(delivered || '(Chưa có dữ liệu giao)');
        $('#order-detail-pre').text(JSON.stringify(data, null, 2));
        $('#copy-data-btn').data('data', delivered);
        $('#copy-memo-btn').data('memo', bankMemo);
        new bootstrap.Modal(document.getElementById('orderModal')).show();
      } catch (e) {
        alertBox('danger', 'Tải chi tiết đơn hàng thất bại: ' + e.message);
      }
    }

    async function markPaid(id) {
      showInputActionModal({
        title: 'Xác nhận thanh toán đơn hàng',
        description: `Điền thông tin thanh toán cho Order ID: ${id}.`,
        confirmText: 'Đánh dấu đã thanh toán',
        fields: [
          {
            name: 'payment_tx_id',
            label: 'payment_tx_id',
            placeholder: 'Mã giao dịch ngân hàng',
            required: true,
            helpText: 'Bắt buộc.',
          },
          {
            name: 'paid_at',
            label: 'paid_at',
            placeholder: '2026-04-13T10:20:30Z',
            required: false,
            helpText: 'RFC3339, ví dụ: 2026-04-13T10:20:30Z.',
          },
        ],
        onSubmit: async (values) => {
          const paidAt = values.paid_at?.trim();
          if (paidAt && Number.isNaN(Date.parse(paidAt))) {
            throw new Error('paid_at không đúng định dạng thời gian hợp lệ (RFC3339).');
          }
          await apiFetch(`/orders/${id}/mark_paid`, {
            method: 'POST',
            body: JSON.stringify({
              payment_tx_id: values.payment_tx_id,
              paid_at: paidAt || null,
            }),
          });
          alertBox('success', I18N.alert.markedPaid);
          await loadOrders();
        },
      });
    }

    async function resendData(id) {
      try {
        await apiFetch(`/orders/${id}/resend`, { method: 'POST', body: '{}' });
        alertBox('success', I18N.alert.resentData);
      } catch (e) {
        alertBox('danger', `${I18N.alert.resendFailed}: ${e.message}`);
      }
    }

    async function cancelOrderAction(id) {
      showConfirmActionModal({
        title: 'Hủy đơn hàng',
        description: 'Bạn có chắc muốn hủy đơn hàng này?',
        context: [`Order ID: ${id}`],
        confirmText: 'Hủy đơn',
        confirmClass: 'btn-danger',
        onConfirm: async () => {
          await apiFetch(`/orders/${id}/cancel`, { method: 'POST', body: JSON.stringify({ reason: 'admin' }) });
          alertBox('success', I18N.alert.cancelledOrder);
          await loadOrders();
        },
      });
    }
