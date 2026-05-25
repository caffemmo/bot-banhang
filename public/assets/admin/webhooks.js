    let webhookOffset = 0;
    async function loadWebhookEvents() {
      const provider = $('#wh-provider').val();
      const memo = $('#wh-memo').val().trim();
      const txid = $('#wh-txid').val().trim();

      const p = provider ? `&provider=${encodeURIComponent(provider)}` : '';
      const m = memo ? `&memo=${encodeURIComponent(memo)}` : '';
      const t = txid ? `&tx_id=${encodeURIComponent(txid)}` : '';

      setFilterLoading('#webhooks-filter-loading', true);
      try {
        const data = await apiFetch(`/webhooks/events?limit=20&offset=${webhookOffset}${p}${m}${t}`);
        const rows = data.items.map(e => {
          const auth = Number(e.authorized) === 1 ? '<span class="badge bg-success">yes</span>' : '<span class="badge bg-danger">no</span>';
          const amt = escapeHtml((e.amount ?? '').toString());
          const order = e.matched_order_id ? `<code>${escapeHtml(String(e.matched_order_id).substring(0, 8))}</code>` : '<span class="text-muted">-</span>';
          return `<tr>
          <td class="mobile-priority"><small>${escapeHtml(e.received_at)}</small></td>
          <td class="mobile-priority"><span class="pill">${escapeHtml(e.provider)}</span></td>
          <td class="d-none d-md-table-cell mobile-hide">${auth}</td>
          <td class="d-none d-lg-table-cell mobile-hide small text-muted">${escapeHtml(e.source_ip || '-')}</td>
          <td class="mobile-priority"><code>${escapeHtml(e.memo_extracted || '-')}</code></td>
          <td class="d-none d-md-table-cell mobile-hide small">${escapeHtml(e.tx_id || '-')}</td>
          <td class="mobile-priority"><span class="fw-bold">${amt}</span></td>
          <td class="d-none d-md-table-cell mobile-hide">${escapeHtml(e.status || '-')}</td>
          <td class="d-none d-lg-table-cell mobile-hide">${order}</td>
          <td class="mobile-priority">${escapeHtml(e.result || '-')}</td>
        </tr>`;
        }).join('');

        $('#webhooks-body').html(rows || `<tr><td colspan="10" class="text-center text-muted">${I18N.common.empty}</td></tr>`);
        $('#webhooks-meta').text(`total=${data.total} · offset=${data.offset} · limit=${data.limit}`);
      } catch (e) {
        alertBox('danger', `${I18N.alert.loadWebhooksFailed}: ${e.message}`);
      } finally {
        setFilterLoading('#webhooks-filter-loading', false);
      }
    }

