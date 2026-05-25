    function bindEvents() {
      $('#confirmActionConfirmBtn').on('click', async function () {
        if (!confirmActionHandler) return;
        const $btn = $(this);
        const idleText = $btn.text();
        try {
          setActionButtonLoading($btn, true, idleText);
          await confirmActionHandler();
          confirmActionModalInstance.hide();
        } catch (e) {
          alertBox('danger', e.message || I18N.alert.actionFailed);
        } finally {
          setActionButtonLoading($btn, false, idleText);
        }
      });

      $('#inputActionForm').on('submit', async function (e) {
        e.preventDefault();
        if (!inputActionHandler?.onSubmit) return;
        const fields = inputActionHandler.fields || [];
        const values = {};
        let hasError = false;
        $('.input-action-field').each(function () {
          const $field = $(this);
          const name = String($field.data('name'));
          const value = String($field.val() || '').trim();
          const required = Number($field.data('required')) === 1;
          values[name] = value;
          if (required && !value) {
            $field.addClass('is-invalid');
            hasError = true;
          } else {
            $field.removeClass('is-invalid');
          }
        });
        if (hasError) {
          alertBox('warning', I18N.alert.requiredFields);
          return;
        }
        const $btn = $('#inputActionConfirmBtn');
        const idleText = inputActionHandler.confirmText || $btn.text();
        try {
          setActionButtonLoading($btn, true, idleText);
          await inputActionHandler.onSubmit(values, fields);
          inputActionModalInstance.hide();
        } catch (err) {
          alertBox('danger', err.message || I18N.alert.actionFailed);
        } finally {
          setActionButtonLoading($btn, false, idleText);
        }
      });

      $('#login-form').on('submit', submitLogin);
      $('#logout-btn').on('click', logout);
      $('#refresh-health').on('click', async () => {
        try {
          const res = await fetch('/api/health');
          if (!res.ok) throw new Error(res.statusText);
          const data = await res.json();
          if (!data.ok) throw new Error('Phản hồi không hợp lệ');
          alertBox('success', `${I18N.alert.apiHealthy}: ${data.data?.status || 'up'}`);
        }
        catch (e) { alertBox('danger', `${I18N.alert.healthFailed}: ${e.message}`); }
      });
      $('#product-delivery-type').on('change', toggleProductDeliveryInputs);

      const debouncedApplyProductFilters = debounce(applyProductFilters, 300);
      const debouncedApplyOrderFilters = debounce(applyOrderFilters, 300);
      const debouncedApplyWalletFilters = debounce(applyWalletFilters, 300);
      const debouncedApplyWebhookFilters = debounce(applyWebhookFilters, 300);

      $('#refresh-products').on('click', applyProductFilters);
      $('#product-query').on('input', debouncedApplyProductFilters);
      $('#product-query').on('keydown', function (e) {
        if (e.key === 'Enter') {
          e.preventDefault();
          applyProductFilters();
        }
      });
      $('#product-active').on('change', applyProductFilters);
      $('#new-product').on('click', () => openProductModal());
      $('#new-product-category').on('click', () => openProductCategoryModal());
      $('#save-product-btn').on('click', saveProduct);
      $('#delete-product-image-btn').on('click', deleteCurrentProductImage);
      document.addEventListener('show.bs.dropdown', (event) => {
        event.target.closest('.admin-action-table-wrap')?.classList.add('dropdown-menu-open');
      });
      document.addEventListener('hidden.bs.dropdown', (event) => {
        event.target.closest('.admin-action-table-wrap')?.classList.remove('dropdown-menu-open');
      });
      $('#products-body').on('click', '.edit-product', async function () {
        const id = $(this).data('id');
        const product = await fetchProduct(id);
        openProductModal(product);
      });
      $('#products-body').on('click', '.add-items', function () {
        openItemsModal($(this).data('id'));
      });
      $('#products-body').on('change', '.product-toggle', function () {
        toggleProduct($(this).data('id'), this.checked ? 1 : 0);
      });
      $('#products-body').on('click', '.delete-product', function () {
        deleteProduct($(this).data('id'));
      });
      $('#products-body').on('click', '.manage-items', function () {
        openItemsList($(this).data('id'), $(this).data('name'));
      });
      $('#products-body').on('click', '.manage-plans', function () {
        openPlans($(this).data('id'), $(this).data('name'));
      });
      $('#product-categories-body').on('click', '.edit-product-category', function () {
        const category = productCategories.find(c => c.id === Number($(this).data('id')));
        if (category) openProductCategoryModal(category);
      });
      $('#product-categories-body').on('click', '.delete-product-category', function () {
        deleteProductCategory($(this).data('id'));
      });

      // Move Up/Down button clicks
      $('#products-body').on('click', '.btn-move-up', function () {
        moveProductUpDown($(this).data('id'), 'up');
      });
      $('#products-body').on('click', '.btn-move-down', function () {
        moveProductUpDown($(this).data('id'), 'down');
      });

      // Drag & Drop product reordering
      $('#products-body').on('dragstart', 'tr', function (e) {
        e.originalEvent.dataTransfer.setData('text/plain', $(this).data('id'));
        $(this).css('opacity', '0.5');
      });
      $('#products-body').on('dragend', 'tr', function (e) {
        $(this).css('opacity', '');
      });
      $('#products-body').on('dragover', 'tr', function (e) {
        e.preventDefault();
      });
      $('#products-body').on('drop', 'tr', function (e) {
        e.preventDefault();
        const draggedId = parseInt(e.originalEvent.dataTransfer.getData('text/plain'));
        const targetId = parseInt($(this).data('id'));
        if (draggedId && targetId && draggedId !== targetId) {
          reorderProductsInList(draggedId, targetId);
        }
      });

      $('#refresh-orders').on('click', applyOrderFilters);
      $('#order-query').on('input', debouncedApplyOrderFilters);
      $('#order-query').on('keydown', function (e) {
        if (e.key === 'Enter') {
          e.preventDefault();
          applyOrderFilters();
        }
      });
      $('#order-status, #order-from, #order-to').on('change', applyOrderFilters);
      $('#export-orders').on('click', exportOrders);
      $('#bc-mode').on('change', toggleBroadcastProductPicker);
      $('#broadcast-tab').on('shown.bs.tab', () => {
        loadBroadcastTemplates();
        toggleBroadcastProductPicker();
      });
      $('#bc-template-select').on('change', function () {
        applyBroadcastTemplate($(this).val());
      });
      $('#bc-template-apply').on('click', () => applyBroadcastTemplate($('#bc-template-select').val()));
      $('#bc-template-save').on('click', saveBroadcastTemplate);
      $('#send-broadcast').on('click', sendBroadcast);
      $('#orders-body').on('click', '.view-order', function () { viewOrder($(this).data('id')); });
      $('#orders-body').on('click', '.mark-paid', function () { markPaid($(this).data('id')); });
      $('#orders-body').on('click', '.resend', function () { resendData($(this).data('id')); });
      $('#orders-body').on('click', '.cancel', function () { cancelOrderAction($(this).data('id')); });

      $('#refresh-wallets').on('click', applyWalletFilters);
      $('#manual-topup-by-id').on('click', () => openManualTopup());
      $('#wallet-query').on('input', debouncedApplyWalletFilters);
      $('#wallet-query').on('keydown', function (e) {
        if (e.key === 'Enter') {
          e.preventDefault();
          applyWalletFilters();
        }
      });
      $('#wallets-prev').on('click', () => { walletOffset = Math.max(0, walletOffset - 20); loadWallets(); });
      $('#wallets-next').on('click', () => { walletOffset += 20; loadWallets(); });
      $('#wallets-tab').on('shown.bs.tab', () => { walletOffset = 0; loadWallets(); });
      $('#wallets-body').on('click', '.view-wallet', function () {
        viewWallet($(this).data('user-id'), $(this).data('user-label'));
      });
      $('#wallets-body').on('click', '.topup-wallet', function () {
        openManualTopup($(this).data('user-id'), $(this).data('user-label'));
      });
      $('#wallets-body').on('click', '.adjust-wallet', function () {
        openWalletAdjust($(this).data('user-id'), $(this).data('user-label'));
      });
      $('#wallet-detail-topup').on('click', function () {
        openManualTopup($(this).data('user-id'), $(this).data('user-label'));
      });

      $('#refresh-webhooks').on('click', applyWebhookFilters);
      $('#webhooks-prev').on('click', () => { webhookOffset = Math.max(0, webhookOffset - 20); loadWebhookEvents(); });
      $('#webhooks-next').on('click', () => { webhookOffset += 20; loadWebhookEvents(); });
      $('#wh-provider').on('change', applyWebhookFilters);
      $('#wh-memo, #wh-txid').on('input', debouncedApplyWebhookFilters);
      $('#wh-memo, #wh-txid').on('keydown', function (e) {
        if (e.key === 'Enter') {
          e.preventDefault();
          applyWebhookFilters();
        }
      });
      $('#webhooks-tab').on('shown.bs.tab', () => { webhookOffset = 0; loadWebhookEvents(); });

      $('#configs-tab').on('shown.bs.tab', () => { loadConfigs(); });
      $('#save-configs-btn').on('click', saveConfigs);
      $('#bot-i18n-tab').on('shown.bs.tab', loadBotI18n);
      $('#bot-i18n-refresh').on('click', loadBotI18n);
      $('#bot-i18n-import-btn').on('click', importBotI18nLanguage);
      $('#bot-i18n-export-btn').on('click', exportBotI18nLanguage);
      $('#bot-i18n-load-editor-btn').on('click', () => loadBotI18nEditor());
      $('#bot-i18n-save-editor-btn').on('click', saveBotI18nEditor);
      $('#bot-i18n-languages-body').on('click', '.bot-i18n-edit', function () {
        loadBotI18nEditor($(this).data('code'));
      });
      $('#admins-tab').on('shown.bs.tab', loadAdmins);
      $('#refresh-admins').on('click', loadAdmins);
      $('#admin-create-form').on('submit', createAdminUser);
      $('#admins-body').on('click', '.change-admin-password', function () {
        openChangeAdminPassword($(this).data('id'), $(this).data('username'));
      });

      $('#copy-data-btn').on('click', function () {
        const dataTxt = $(this).data('data') || '';
        navigator.clipboard.writeText(dataTxt).then(() => alertBox('success', I18N.alert.copiedDeliveredData));
      });
      $('#copy-memo-btn').on('click', function () {
        const memo = $(this).data('memo') || '';
        navigator.clipboard.writeText(memo).then(() => alertBox('success', I18N.alert.copiedMemo));
      });

      $('#save-items-btn').on('click', saveItems);
      $('#items-prev').on('click', () => { itemsOffset = Math.max(0, itemsOffset - 20); loadItemsList(); });
      $('#items-next').on('click', () => { itemsOffset += 20; loadItemsList(); });
      $('#items-body').on('click', '.copy-item', function () {
        const content = $(this).data('content') || '';
        navigator.clipboard.writeText(content).then(() => alertBox('success', I18N.alert.copiedItem));
      });
      $('#items-body').on('click', '.delete-item', function () {
        deleteItem($(this).data('id'));
      });

      $('#save-plan-btn').on('click', savePlan);
      $('#reset-plan-btn').on('click', resetPlanForm);
      $('#plans-body').on('click', '.edit-plan', function () {
        const id = $(this).data('id');
        const plan = {
          label: $(this).data('label'),
          months: $(this).data('months'),
          price: $(this).data('price'),
          sort_order: $(this).data('sort'),
        };
        editingPlanId = id;
        $('#plan-label').val(plan.label);
        $('#plan-months').val(plan.months);
        $('#plan-price').val(plan.price);
        $('#plan-sort').val(plan.sort_order ?? 0);
      });
      $('#plans-body').on('click', '.delete-plan', function () {
        deletePlan($(this).data('id'));
      });

      $('#refresh-rev-7d').on('click', () => loadRevenue(7, '#rev-7d', '#rev-7d-range'));
      $('#refresh-rev-30d').on('click', () => loadRevenue(30, '#rev-30d', '#rev-30d-range'));
    }

