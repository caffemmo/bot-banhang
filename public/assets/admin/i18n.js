    let botI18nLanguages = [];
    let currentBotI18nDetail = null;
    const I18N_KEY_GROUPS = [
      {
        title: 'Chung',
        match: key => [
          'callback_ack', 'action_invalid', 'error', 'unauthorized', 'session_expired',
          'user_unknown', 'user_unknown_retry', 'back_btn', 'fallback_default', 'ping_pong',
        ].includes(key),
      },
      {
        title: 'Khởi động & ngôn ngữ',
        match: key => key === 'start'
          || key === 'help'
          || key.startsWith('start_btn_')
          || key.startsWith('language_'),
      },
      {
        title: 'Kênh bắt buộc',
        match: key => key.startsWith('required_channel_'),
      },
      {
        title: 'Thông báo broadcast',
        match: key => key.startsWith('broadcast_'),
      },
      {
        title: 'Admin & thông báo nội bộ',
        match: key => key.startsWith('admin_'),
      },
      {
        title: 'Shop & sản phẩm',
        match: key => [
          'cmd_shop', 'open_shop_btn', 'no_products', 'shop_page_title',
          'pagination_next', 'pagination_prev', 'product_not_found', 'not_found_product',
          'default_input_prompt', 'manual_product_plan_prompt',
        ].includes(key)
          || key.startsWith('shop_')
          || key.startsWith('product_')
          || key.startsWith('stock_')
          || key.startsWith('uploaded_file_'),
      },
      {
        title: 'Số lượng, gói & thông tin',
        match: key => key.startsWith('plan_')
          || key.startsWith('info_')
          || ['buy_prompt_qty', 'qty_invalid', 'invalid_qty'].includes(key),
      },
      {
        title: 'Đơn hàng, thanh toán & giao hàng',
        match: key => key.startsWith('order_')
          || key.startsWith('delivery_')
          || key.startsWith('cancel_')
          || ['cmd_orders', 'no_orders', 'payment_pending', 'paywallet_btn', 'qr_countdown', 'qr_expired', 'continue_shopping_btn', 'cancel_order_btn'].includes(key),
      },
      {
        title: 'Ví & nạp tiền',
        match: key => key.startsWith('wallet_') || key.startsWith('topup_'),
      },
    ];

    function groupForI18nKey(key) {
      const index = I18N_KEY_GROUPS.findIndex(group => group.match(key));
      if (index >= 0) {
        return { index, title: I18N_KEY_GROUPS[index].title };
      }
      return { index: I18N_KEY_GROUPS.length, title: 'Khác' };
    }

    function groupedI18nKeys(keys) {
      const uniqueKeys = Array.from(new Set(keys || []));
      uniqueKeys.sort((left, right) => {
        const leftGroup = groupForI18nKey(left);
        const rightGroup = groupForI18nKey(right);
        if (leftGroup.index !== rightGroup.index) return leftGroup.index - rightGroup.index;
        return left.localeCompare(right);
      });
      const groups = [];
      for (const key of uniqueKeys) {
        const group = groupForI18nKey(key);
        let bucket = groups[groups.length - 1];
        if (!bucket || bucket.title !== group.title) {
          bucket = { title: group.title, keys: [] };
          groups.push(bucket);
        }
        bucket.keys.push(key);
      }
      return groups;
    }

    const KEYBOARD_BUTTON_I18N_KEYS = new Set([
      'back_btn',
      'cancel_crypto_btn',
      'cancel_order_btn',
      'check_crypto_btn',
      'continue_shopping_btn',
      'copy_address_btn',
      'copy_amount_btn',
      'language_btn_en',
      'language_btn_vi',
      'open_shop_btn',
      'pagination_next',
      'pagination_prev',
      'pay_bep20_btn',
      'pay_binance_btn',
      'paywallet_btn',
      'required_channel_check_btn',
      'required_channel_join_btn',
      'shop_back_btn',
      'start_btn_api_integration',
      'start_btn_help',
      'start_btn_language',
      'start_btn_orders',
      'start_btn_purchased',
      'start_btn_shop',
      'start_btn_topup',
      'start_btn_topup_history',
      'start_btn_wallet',
      'stock_notify_buy_now_btn',
      'stock_notify_view_more_btn',
    ]);

    function isKeyboardButtonI18nKey(key) {
      return KEYBOARD_BUTTON_I18N_KEYS.has(key)
        || key.startsWith('start_btn_')
        || key.startsWith('shop_btn_')
        || key.startsWith('topup_btn_')
        || key.startsWith('wallet_btn_');
    }

    function emojiConfigForKey(detail, key) {
      const config = detail?.emojis?.[key] || {};
      const first = Array.isArray(config.emojis) && config.emojis.length ? config.emojis[0] : {};
      return {
        fallback: config.fallback || first.fallback || '',
        custom_emoji_id: config.custom_emoji_id || first.custom_emoji_id || '',
      };
    }

    function leadingEmojiToken(value) {
      const trimmed = String(value || '').trim();
      if (!trimmed) return '';
      const firstToken = trimmed.split(/\s+/)[0] || '';
      return /[\p{L}\p{N}]/u.test(firstToken) ? '' : firstToken.slice(0, 16);
    }

    function renderKeyboardButtonEmojiFields(detail, key, value, fallbackValue) {
      if (!isKeyboardButtonI18nKey(key)) return '';
      const config = emojiConfigForKey(detail, key);
      const suggestedFallback = leadingEmojiToken(value) || leadingEmojiToken(fallbackValue) || '';
      const fallbackPlaceholder = leadingEmojiToken(value) || leadingEmojiToken(fallbackValue) || 'VD: 🛒';
      return `
        <div class="row g-2 mt-2 bot-i18n-emoji-row" data-key="${escapeAttr(key)}" data-suggested-fallback="${escapeAttr(suggestedFallback)}">
          <div class="col-12 col-md-4">
            <label class="form-label small mb-1">Emoji thường</label>
            <input class="form-control form-control-sm bot-i18n-emoji-fallback" data-key="${escapeAttr(key)}"
              maxlength="16" value="${escapeAttr(config.fallback)}" placeholder="${escapeAttr(fallbackPlaceholder)}">
          </div>
          <div class="col-12 col-md-8">
            <label class="form-label small mb-1">Custom emoji ID nút</label>
            <input class="form-control form-control-sm bot-i18n-emoji-custom-id" data-key="${escapeAttr(key)}"
              inputmode="numeric" maxlength="64" value="${escapeAttr(config.custom_emoji_id)}"
              placeholder="VD: 6172437452590944785">
          </div>
        </div>
      `;
    }

    function buildBotI18nEmojiPayload() {
      const emojis = {};
      const current = currentBotI18nDetail?.emojis || {};
      Object.keys(current).forEach(key => {
        emojis[key] = current[key];
      });

      $('.bot-i18n-emoji-row').each(function () {
        const key = String($(this).data('key') || '');
        const explicitFallback = String($(this).find('.bot-i18n-emoji-fallback').val() || '').trim();
        const textFallback = leadingEmojiToken($(this).closest('.bot-i18n-field').find('.bot-i18n-input').val());
        const suggestedFallback = String($(this).data('suggested-fallback') || '').trim();
        const fallback = explicitFallback || textFallback || suggestedFallback;
        const customId = String($(this).find('.bot-i18n-emoji-custom-id').val() || '').trim();
        if (!key) return;
        if (!fallback && !customId) {
          delete emojis[key];
          return;
        }
        if (customId && !/^\d{8,64}$/.test(customId)) {
          throw new Error(`Custom emoji ID nút ${key} phải là số, dài 8-64 ký tự.`);
        }
        if (!fallback) {
          throw new Error(`Nhập Emoji thường cho ${key} để làm fallback khi Telegram không hiện emoji động.`);
        }
        emojis[key] = customId
          ? { fallback, custom_emoji_id: customId }
          : fallback;
      });

      return emojis;
    }

    async function loadBotI18n() {
      await loadBotI18nLanguages();
    }

    async function loadBotI18nLanguages() {
      try {
        botI18nLanguages = await apiFetch('/i18n/languages');
        renderBotI18nLanguageList(botI18nLanguages);
        renderBotI18nLanguageOptions(botI18nLanguages);
      } catch (e) {
        alertBox('danger', 'Tải danh sách ngôn ngữ thất bại: ' + e.message);
      }
    }

    function renderBotI18nLanguageList(languages) {
      const rows = languages.map(lang => `
        <tr>
          <td class="font-monospace">${escapeHtml(lang.code)}</td>
          <td>${escapeHtml(lang.label)}</td>
          <td class="font-monospace">${escapeHtml(lang.fallback)}</td>
          <td>${lang.enabled ? '<span class="badge text-bg-success">Bật</span>' : '<span class="badge text-bg-secondary">Tắt</span>'}</td>
          <td>${Number(lang.key_count || 0).toLocaleString('vi-VN')}</td>
          <td>
            <button class="btn btn-sm btn-outline-primary bot-i18n-edit" data-code="${escapeAttr(lang.code)}">Sửa</button>
          </td>
        </tr>
      `).join('');
      $('#bot-i18n-languages-body').html(rows || `<tr><td colspan="6" class="text-center text-muted">${I18N.common.empty}</td></tr>`);
    }

    function renderBotI18nLanguageOptions(languages) {
      const options = languages.map(lang => `<option value="${escapeAttr(lang.code)}">${escapeHtml(lang.label)} (${escapeHtml(lang.code)})</option>`).join('');
      $('#bot-i18n-export-code, #bot-i18n-edit-code').html(options);
    }

    async function importBotI18nLanguage() {
      const format = $('#bot-i18n-import-format').val();
      const content = $('#bot-i18n-import-content').val();
      if (!content.trim()) {
        alertBox('warning', 'Dán nội dung JSON/YAML trước khi import.');
        return;
      }

      try {
        const result = await apiFetch('/i18n/import', {
          method: 'POST',
          body: JSON.stringify({ format, content }),
        });
        alertBox('success', `Đã import ${result.imported_keys} key cho ${result.language.label}.`);
        $('#bot-i18n-import-content').val('');
        await loadBotI18nLanguages();
      } catch (e) {
        alertBox('danger', 'Import ngôn ngữ thất bại: ' + e.message);
      }
    }

    async function exportBotI18nLanguage() {
      const code = $('#bot-i18n-export-code').val();
      if (!code) return;

      try {
        const data = await apiFetch(`/i18n/export/${encodeURIComponent(code)}`);
        $('#bot-i18n-export-output').val(JSON.stringify(data, null, 2));
      } catch (e) {
        alertBox('danger', 'Export ngôn ngữ thất bại: ' + e.message);
      }
    }

    async function loadBotI18nEditor(code) {
      const selectedCode = code || $('#bot-i18n-edit-code').val();
      if (!selectedCode) return;

      $('#bot-i18n-edit-code').val(selectedCode);
      $('#bot-i18n-editor-title').text(`Đang tải bản dịch ${selectedCode}...`);
      $('#bot-i18n-editor-fields').html('<div class="text-muted">Đang tải...</div>');
      document.getElementById('bot-i18n-editor-card')?.scrollIntoView({ behavior: 'smooth', block: 'start' });

      try {
        currentBotI18nDetail = await apiFetch(`/i18n/language/${encodeURIComponent(selectedCode)}`);
        $('#bot-i18n-edit-code').val(currentBotI18nDetail.language.code);
        renderBotI18nEditor(currentBotI18nDetail);
        document.getElementById('bot-i18n-editor-card')?.scrollIntoView({ behavior: 'smooth', block: 'start' });
      } catch (e) {
        currentBotI18nDetail = null;
        alertBox('danger', 'Tải bản dịch thất bại: ' + e.message);
      }
    }

    function renderBotI18nEditor(detail) {
      const keyGroups = groupedI18nKeys(detail.keys || []);
      const bot = detail.bot || {};
      const fallback = detail.fallback_bot || {};
      const emojisEnabled = detail.emojis_enabled === true;
      const html = keyGroups.map(group => {
        const fields = group.keys.map(key => {
          const value = bot[key] ?? '';
          const fallbackValue = fallback[key] ?? '';
          const rows = String(value || fallbackValue).includes('\n') || String(value || fallbackValue).length > 70 ? 4 : 2;
          return `
            <div class="col-12 col-lg-6 bot-i18n-field">
              <label class="form-label font-monospace small mb-1">${escapeHtml(key)}</label>
              <textarea class="form-control bot-i18n-input" data-key="${escapeAttr(key)}" rows="${rows}">${escapeHtml(value)}</textarea>
              ${renderKeyboardButtonEmojiFields(detail, key, value || fallbackValue, fallbackValue)}
              ${fallbackValue ? `<div class="form-text text-muted">Fallback: ${escapeHtml(fallbackValue)}</div>` : ''}
              ${emojisEnabled ? '<div class="form-text">Để đặt emoji động đúng vị trí, nhập {Custom emoji ID} trong nội dung text.</div>' : ''}
            </div>
          `;
        }).join('');
        return `
          <div class="col-12">
            <div class="fw-bold border-bottom pb-2 mt-2">${escapeHtml(group.title)}</div>
          </div>
          ${fields}
        `;
      }).join('');

      $('#bot-i18n-editor-title').text(`${detail.language.label} (${detail.language.code})`);
      $('#bot-i18n-editor-fields').html(html || '<div class="text-muted">Chưa có key bản dịch.</div>');
      $('#bot-i18n-editor-card').toggleClass('d-none', false);
    }

    async function saveBotI18nEditor() {
      if (!currentBotI18nDetail) return;
      const bot = {};
      $('.bot-i18n-input').each(function () {
        bot[$(this).data('key')] = $(this).val();
      });

      try {
        const code = currentBotI18nDetail.language.code;
        const payload = { bot };
        payload.emojis = buildBotI18nEmojiPayload();
        const result = await apiFetch(`/i18n/language/${encodeURIComponent(code)}`, {
          method: 'PUT',
          body: JSON.stringify(payload),
        });
        alertBox('success', `Đã lưu ${result.imported_keys} key cho ${result.language.label}.`);
        await loadBotI18nLanguages();
      } catch (e) {
        alertBox('danger', 'Lưu bản dịch thất bại: ' + e.message);
      }
    }
