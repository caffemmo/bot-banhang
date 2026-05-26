    const BOT_OPERATIONAL_CONFIGS = [
      {
        title: 'Cửa hàng',
        icon: '⚙️',
        fields: [
          { key: 'base_url', label: 'Base URL', value: '' },
          { key: 'stock_auto_broadcast_enabled', label: 'Tự động thông báo khi nhập kho', value: '0' },
        ]
      },
      {
        title: 'Ngân hàng',
        icon: '🏦',
        fields: [
          { key: 'bank_name', label: 'Ngân hàng', value: '' },
          { key: 'bank_account', label: 'Số tài khoản', value: '' },
          { key: 'bank_account_name', label: 'Tên chủ tài khoản', value: '' },
          { key: 'order_memo_prefix', label: 'Prefix nội dung CK đơn hàng', value: 'PTN1411' },
          { key: 'order_memo_length', label: 'Số ký tự random mã CK đơn hàng', value: '10' },
        ]
      },
      {
        title: 'Kênh Telegram',
        icon: '📢',
        fields: [
          { key: 'required_channel_enabled', label: 'Bật yêu cầu tham gia channel', value: '1' },
          { key: 'required_channel_id', label: 'Channel ID / username', value: '@xxxx' },
          { key: 'required_channel_url', label: 'Link channel', value: 'https://t.me/xxxx' },
        ]
      },
      {
        title: 'Admin Telegram',
        icon: '🧩',
        fields: [
          { key: 'telegram_icon_admin_ids', label: 'Admin IDs được lấy thông tin media Telegram', value: '' },
          { key: 'order_notifications_enabled', label: 'Bật thông báo đơn thanh toán thành công', value: '0' },
          { key: 'order_notification_admin_ids', label: 'Admin IDs nhận thông báo đơn hàng', value: '' },
        ]
      },
      {
        title: 'Emoji Telegram',
        icon: '⭐',
        fields: [
          { key: 'telegram_i18n_emojis_enabled', label: 'Bật emoji trước text i18n', value: '0' },
        ]
      },
      {
        title: 'USDT chung & tỷ giá',
        icon: '💵',
        fields: [
          { key: 'usd_vnd_fallback_rate', label: 'Tỷ giá USDT/VND fallback', value: '25000' },
          { key: 'usdt_rate_custom_url', label: 'URL tỷ giá USDT/VND JSON', value: '' },
          { key: 'usdt_rate_buffer_percent', label: 'Buffer tỷ giá USDT %', value: '1' },
          { key: 'usdt_rate_cache_seconds', label: 'Cache tỷ giá giây', value: '300' },
          { key: 'usdt_rate_stale_seconds', label: 'Dùng cache stale giây', value: '600' },
          { key: 'crypto_pay_ttl_minutes', label: 'USDT TTL phút', value: '30' },
        ]
      },
      {
        title: 'Thanh toán BEP20',
        icon: '⛓️',
        fields: [
          { key: 'bep20_merchant_wallet', label: 'Ví nhận USDT BEP20', value: '' },
          { key: 'bep20_usdt_contract', label: 'USDT contract BEP20', value: '0x55d398326f99059fF775485246999027B3197955' },
          { key: 'bep20_required_confirmations', label: 'Số confirmation BEP20', value: '12' },
          { key: 'bep20_start_block', label: 'BEP20 start block', value: '' },
          { key: 'bscscan_api_key', label: 'Etherscan API V2 key (BNB chain)', value: '' },
        ]
      },
      {
        title: 'Binance Pay',
        icon: '💳',
        fields: [
          { key: 'binance_pay_note_enabled', label: 'Bật Binance Pay note reconciliation (1/0)', value: '0' },
          { key: 'binance_pay_api_key', label: 'Binance Pay API key', value: '' },
          { key: 'binance_pay_api_secret', label: 'Binance Pay API secret', value: '' },
          { key: 'binance_pay_receiver_pay_id', label: 'Pay ID nhận tiền', value: '' },
          { key: 'binance_pay_receiver_name', label: 'Binance ID / tên nhận', value: '' },
          { key: 'binance_pay_poll_interval_seconds', label: 'Chu kỳ quét Binance Pay (giây)', value: '30' },
          { key: 'binance_pay_history_lookback_minutes', label: 'Cửa sổ lịch sử Binance Pay (phút)', value: '120' },
          { key: 'binance_pay_note_prefix', label: 'Prefix ghi chú', value: 'VI' },
          { key: 'binance_pay_note_digits', label: 'Số chữ số ghi chú', value: '6' },
          { key: 'binance_pay_secret', label: 'Legacy Merchant secret (không dùng flow mới)', value: '' },
          { key: 'binance_pay_cert_sn', label: 'Legacy Merchant cert SN (không dùng flow mới)', value: '' },
        ]
      }
    ];

    async function loadConfigs() {
      try {
        const configs = await apiFetch(`/configs`);
        const container = $('#configs-form-container');
        const navHtml = BOT_OPERATIONAL_CONFIGS.map((section, index) => `
          <a class="config-section-link" href="#config-section-${index}">
            <span class="config-section-link-icon">${section.icon}</span>
            <span class="config-section-link-title">${escapeHtml(section.title)}</span>
            <span class="config-section-link-count">${section.fields.length}</span>
          </a>
        `).join('');
        const sectionsHtml = BOT_OPERATIONAL_CONFIGS.map((section, index) => {
          const fieldsHtml = section.fields.map(item => {
            const value = configs[item.key] ?? item.value ?? '';
            return buildConfigInput(item.key, item.label, value);
          }).join('');
          
          return `
            <section class="config-section-card" id="config-section-${index}">
              <div class="config-section-header">
                <div class="config-section-title-wrap">
                  <span class="config-section-icon">${section.icon}</span>
                  <h6 class="config-section-title">${escapeHtml(section.title)}</h6>
                </div>
                <span class="config-section-meta">${section.fields.length} mục</span>
              </div>
              <div class="row g-3">
                ${fieldsHtml}
              </div>
            </section>
          `;
        }).join('');
        container.html(`
          <div class="config-layout">
            <aside class="config-nav" aria-label="Nhóm cấu hình Bot">
              ${navHtml}
            </aside>
            <div class="config-section-stack">
              ${sectionsHtml}
            </div>
          </div>
        `);
      } catch (e) {
        alertBox('danger', 'Tải cấu hình thất bại: ' + e.message);
      }
    }

    function buildConfigInput(key, label, value) {
      const inputHtml = buildConfigFieldHtml(key, value);
      return `
      <div class="col-md-6 col-12">
        <label class="form-label small mb-1">${escapeHtml(label)} <span class="font-monospace text-muted" style="font-size: 10px;">(${escapeHtml(key)})</span></label>
        ${inputHtml}
      </div>
    `;
    }

    function buildConfigFieldHtml(key, value) {
      value = value == null ? '' : String(value);
      if (key === 'required_channel_enabled' || key === 'telegram_i18n_emojis_enabled' || key === 'stock_auto_broadcast_enabled' || key === 'order_notifications_enabled') {
        const isSelected1 = value === '1' ? 'selected' : '';
        const isSelected0 = value === '0' ? 'selected' : '';
        return `
          <select class="form-select config-input" data-key="${escapeAttr(key)}">
            <option value="1" ${isSelected1}>Bật</option>
            <option value="0" ${isSelected0}>Tắt</option>
          </select>
        `;
      }
      if (key === 'telegram_icon_admin_ids' || key === 'order_notification_admin_ids') {
        return `
          <textarea class="form-control config-input" data-key="${escapeAttr(key)}" rows="3"
            placeholder="123456789, 987654321">${escapeHtml(value)}</textarea>
          <div class="form-text">Nhập Telegram user ID của admin, cách nhau bằng dấu phẩy hoặc xuống dòng.</div>
        `;
      }
      if (key === 'order_memo_prefix') {
        return `
          <input type="text" class="form-control config-input" data-key="${escapeAttr(key)}"
            value="${escapeAttr(value)}" maxlength="10" autocomplete="off">
          <div class="form-text">Chỉ dùng chữ và số, tối đa 10 ký tự. Không dùng NAP vì dành cho nạp ví.</div>
        `;
      }
      if (key === 'order_memo_length') {
        return `
          <input type="number" class="form-control config-input" data-key="${escapeAttr(key)}"
            value="${escapeAttr(value)}" min="10" max="16" step="1">
          <div class="form-text">Độ dài phần random sau prefix, từ 10 đến 16 ký tự.</div>
        `;
      }
      const isTextarea = value.includes('\n') || value.length > 70;
      return isTextarea
        ? `<textarea class="form-control config-input" data-key="${escapeAttr(key)}" rows="3">${escapeHtml(value)}</textarea>`
        : `<input type="text" class="form-control config-input" data-key="${escapeAttr(key)}" value="${escapeAttr(value)}">`;
    }

    async function saveConfigs() {
      const payload = {};
      $('.config-input').each(function () {
        payload[$(this).data('key')] = $(this).val();
      });

      try {
        const validationError = validateConfigsPayload(payload);
        if (validationError) {
          alertBox('danger', validationError);
          return;
        }
        await apiFetch(`/configs`, { method: 'POST', body: JSON.stringify(payload) });
        alertBox('success', I18N.alert.saveConfigsSuccess);
      } catch (e) {
        alertBox('danger', 'Lưu cấu hình thất bại: ' + e.message);
      }
    }

    function validateConfigsPayload(payload) {
      if (Object.prototype.hasOwnProperty.call(payload, 'order_memo_prefix')) {
        const prefix = String(payload.order_memo_prefix || '').trim().toUpperCase();
        if (!/^[A-Z0-9]{1,10}$/.test(prefix)) {
          return 'Prefix nội dung CK đơn hàng phải là chữ/số, dài 1-10 ký tự.';
        }
        if ('NAP'.startsWith(prefix) || prefix.startsWith('NAP')) {
          return 'Prefix nội dung CK đơn hàng không được trùng prefix nạp ví NAP.';
        }
        payload.order_memo_prefix = prefix;
      }

      if (Object.prototype.hasOwnProperty.call(payload, 'order_memo_length')) {
        const rawLength = String(payload.order_memo_length || '').trim();
        const length = Number(rawLength);
        if (!Number.isInteger(length) || length < 10 || length > 16) {
          return 'Số ký tự random mã CK đơn hàng phải nằm trong khoảng 10 đến 16.';
        }
        payload.order_memo_length = String(length);
      }

      return null;
    }
