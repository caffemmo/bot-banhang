    const API_BASE = '/api/admin';
    const I18N = {
      common: { empty: 'Trống', processing: 'Đang xử lý...', confirm: 'Xác nhận' },
      status: { yes: 'Có', no: 'Không' },
      alert: {
        requestFailed: 'Yêu cầu thất bại',
        loadProductsFailed: 'Tải danh sách sản phẩm thất bại',
        loadOrdersFailed: 'Tải danh sách đơn hàng thất bại',
        loadWalletsFailed: 'Tải danh sách ví thất bại',
        loadWebhooksFailed: 'Tải sự kiện webhook thất bại',
        exportFailed: 'Xuất file thất bại',
        saveFailed: 'Lưu thất bại',
        markedPaid: 'Đã đánh dấu đơn hàng đã thanh toán.',
        tokenSaved: 'Đã lưu token.',
        healthFailed: 'Kiểm tra health thất bại',
        apiHealthy: 'API hoạt động ổn định',
        requiredBroadcastContent: 'Nhập nội dung thông báo.',
        requiredBroadcastProduct: 'Chọn sản phẩm cần thông báo.',
        broadcastSent: 'Đã gửi thông báo.',
        broadcastQueued: 'Đã đưa thông báo vào hàng gửi nền.',
        broadcastFailed: 'Gửi thất bại',
        loadBroadcastProductsFailed: 'Tải sản phẩm cho thông báo thất bại',
        savedProduct: 'Lưu sản phẩm thành công.',
        needOneItem: 'Nhập ít nhất 1 dòng',
        addItemsSuccess: 'Đã thêm {count} item',
        addItemsFailed: 'Thêm item thất bại',
        uploadFileFailed: 'Upload file giao hàng thất bại',
        loadCategoriesFailed: 'Tải danh mục thất bại',
        savedCategory: 'Đã lưu danh mục.',
        deletedCategory: 'Đã ẩn danh mục.',
        deletedProductImage: 'Đã xóa ảnh sản phẩm.',
        deleteProductImageFailed: 'Xóa ảnh sản phẩm thất bại',
        deleteItemSuccess: 'Đã xóa item',
        planUpdated: 'Cập nhật gói thành công.',
        planAdded: 'Thêm gói thành công.',
        savePlanFailed: 'Lưu gói thất bại',
        deletePlanSuccess: 'Đã xóa gói',
        updatedStatus: 'Cập nhật trạng thái thành công.',
        toggleFailed: 'Cập nhật trạng thái thất bại',
        saveConfigsSuccess: 'Đã lưu cấu hình Bot thành công!',
        disabledProduct: 'Đã ngưng bán sản phẩm',
        resentData: 'Đã gửi lại dữ liệu',
        resendFailed: 'Gửi lại dữ liệu thất bại',
        cancelledOrder: 'Đã hủy đơn',
        actionFailed: 'Thao tác thất bại.',
        requiredFields: 'Vui lòng nhập đủ các trường bắt buộc.',
        copiedDeliveredData: 'Đã sao chép dữ liệu giao',
        copiedMemo: 'Đã sao chép memo',
        copiedItem: 'Đã sao chép item',
        uploadImageFailed: 'Đã lưu thông tin sản phẩm nhưng upload ảnh thất bại. Bạn có thể thử lại ảnh sau.',
        walletTopupSuccess: 'Đã nạp tiền vào ví.',
        walletAdjustSuccess: 'Đã điều chỉnh ví.',
      },
    };
    let productOffset = 0, orderOffset = 0, walletOffset = 0;
    let itemsOffset = 0, currentItemProduct = null;
    let currentPlanProduct = null, editingPlanId = null;
    let confirmActionModalInstance = null;
    let inputActionModalInstance = null;
    let confirmActionHandler = null;
    let inputActionHandler = null;
    let needsSetup = false;
    let currentAdminUser = null;
    let currentWalletUserId = null;
    let allLoadedProducts = [];
    let productCategories = [];

    function escapeHtml(value) {
      return String(value ?? '')
        .replace(/&/g, '&amp;')
        .replace(/</g, '&lt;')
        .replace(/>/g, '&gt;')
        .replace(/"/g, '&quot;')
        .replace(/'/g, '&#39;');
    }

    function escapeAttr(value) {
      return escapeHtml(value);
    }

    function alertBox(type, msg) {
      const html = `<div class="alert alert-${type} alert-dismissible fade show" role="alert">
      ${escapeHtml(msg)}
      <button type="button" class="btn-close" data-bs-dismiss="alert" aria-label="Đóng"></button>
    </div>`;
      $('#alert-area').html(html);
    }

    async function authFetch(path, options = {}) {
      const headers = Object.assign({ 'Content-Type': 'application/json' }, options.headers || {});
      const res = await fetch(path, Object.assign({}, options, { headers, credentials: 'same-origin' }));
      const data = await res.json().catch(() => null);
      if (!res.ok || (data && data.ok === false)) {
        const msg = data?.error?.message || res.statusText || I18N.alert.requestFailed;
        throw new Error(msg);
      }
      return data?.data ?? data;
    }

    function setAuthenticated(user) {
      currentAdminUser = user || null;
      $('.auth-required').toggleClass('d-none', !user);
      $('#auth-panel').toggleClass('d-none', !!user);
      $('#logout-btn, #admin-user-badge').toggleClass('d-none', !user);
      $('#admin-user-badge').text(user ? `Admin: ${user.username}` : '');
    }

    async function checkAuth() {
      try {
        const data = await authFetch('/api/auth/me', { method: 'GET' });
        setAuthenticated(data.user);
        await loadInitialData();
      } catch (_) {
        setAuthenticated(null);
        const status = await authFetch('/api/auth/setup-status', { method: 'GET' });
        needsSetup = !!status.needs_setup;
        $('#auth-title').text(needsSetup ? 'Tạo admin đầu tiên' : 'Đăng nhập quản trị');
        $('#login-submit').text(needsSetup ? 'Tạo admin' : 'Đăng nhập');
        $('#setup-code-wrap').toggleClass('d-none', !needsSetup);
      }
    }

    async function submitLogin(e) {
      e.preventDefault();
      const $btn = $('#login-submit');
      let idleText = $btn.text();
      try {
        setActionButtonLoading($btn, true, idleText);
        const payload = {
          username: $('#login-username').val().trim(),
          password: $('#login-password').val(),
        };
        const wasSetup = needsSetup;
        if (wasSetup) {
          payload.setup_code = $('#setup-code').val().trim();
        }
        const path = wasSetup ? '/api/auth/setup' : '/api/auth/login';
        const data = await authFetch(path, { method: 'POST', body: JSON.stringify(payload) });
        
        $('#login-password, #setup-code').val('');
        
        if (wasSetup) {
          needsSetup = false;
          $('#auth-title').text('Đăng nhập quản trị');
          idleText = 'Đăng nhập';
          $('#setup-code-wrap').addClass('d-none');
          alertBox('success', 'Đã tạo admin đầu tiên thành công. Vui lòng đăng nhập.');
        } else {
          setAuthenticated(data.user);
          alertBox('success', 'Đăng nhập thành công.');
          await loadInitialData();
        }
      } catch (err) {
        alertBox('danger', err.message || 'Thao tác thất bại.');
      } finally {
        setActionButtonLoading($btn, false, idleText);
      }
    }

    async function logout() {
      await authFetch('/api/auth/logout', { method: 'POST', body: '{}' });
      setAuthenticated(null);
      await checkAuth();
    }

    async function loadInitialData() {
      await Promise.allSettled([
        loadRevenue(7, '#rev-7d', '#rev-7d-range'),
        loadRevenue(30, '#rev-30d', '#rev-30d-range'),
        loadProducts(),
        loadOrders(),
        loadWallets(),
        loadWebhookEvents(),
        loadAdmins(),
      ]);
    }

    function ensureActionModalsInit() {
      if (!confirmActionModalInstance) {
        confirmActionModalInstance = new bootstrap.Modal(document.getElementById('confirmActionModal'));
      }
      if (!inputActionModalInstance) {
        inputActionModalInstance = new bootstrap.Modal(document.getElementById('inputActionModal'));
      }
    }

    function setActionButtonLoading($btn, isLoading, idleText, loadingText = I18N.common.processing) {
      $btn.prop('disabled', isLoading);
      $btn.text(isLoading ? loadingText : idleText);
    }

    function showConfirmActionModal({ title, description, context = [], confirmText = I18N.common.confirm, confirmClass = 'btn-danger', onConfirm }) {
      ensureActionModalsInit();
      const $title = $('#confirmActionTitle');
      const $description = $('#confirmActionDescription');
      const $context = $('#confirmActionContext');
      const $confirmBtn = $('#confirmActionConfirmBtn');

      $title.text(title || 'Xác nhận thao tác');
      $description.text(description || '');
      $context.html((context || []).map(item => `<li>${escapeHtml(item)}</li>`).join(''));
      $confirmBtn.removeClass('btn-danger btn-primary btn-warning btn-success').addClass(confirmClass).text(confirmText);
      setActionButtonLoading($confirmBtn, false, confirmText);
      confirmActionHandler = onConfirm;
      confirmActionModalInstance.show();
    }

    function renderInputActionFields(fields) {
      const html = fields.map((field, index) => `
      <div>
        <label class="form-label" for="input-action-field-${index}">${escapeHtml(field.label)}${field.required ? ' <span class="text-danger">*</span>' : ''}</label>
        <input
          id="input-action-field-${index}"
          class="form-control input-action-field"
          type="${escapeAttr(field.type || 'text')}"
          data-name="${escapeAttr(field.name)}"
          data-required="${field.required ? 1 : 0}"
          ${field.maxlength ? `maxlength="${escapeAttr(field.maxlength)}"` : ''}
          value="${escapeAttr(field.value || '')}"
          placeholder="${escapeAttr(field.placeholder || '')}"
        >
        ${field.helpText ? `<small class="text-muted">${escapeHtml(field.helpText)}</small>` : ''}
      </div>
    `).join('');
      $('#inputActionFields').html(html);
    }

    function showInputActionModal({ title, description, fields = [], confirmText = I18N.common.confirm, onSubmit }) {
      ensureActionModalsInit();
      $('#inputActionTitle').text(title || 'Nhập thông tin');
      $('#inputActionDescription').text(description || '');
      renderInputActionFields(fields);
      $('#inputActionConfirmBtn').text(confirmText);
      setActionButtonLoading($('#inputActionConfirmBtn'), false, confirmText);
      inputActionHandler = { fields, onSubmit, confirmText };
      inputActionModalInstance.show();
    }

    async function loadRevenue(days, valueEl, rangeEl) {
      try {
        const data = await apiFetch(`/stats/revenue?days=${days}`);
        $(valueEl).text(data.amount.toLocaleString('vi-VN') + ' ₫');
        const from = new Date(data.from).toLocaleString('vi-VN');
        const to = new Date(data.to).toLocaleString('vi-VN');
        $(rangeEl).text(`${from} → ${to}`);
      } catch (e) {
        $(valueEl).text('--');
        $(rangeEl).text('');
      }
    }

    async function loadBroadcastProducts() {
      try {
        const data = await apiFetch('/products?limit=100&offset=0&active=1');
        const items = data.items || [];
        const options = items.map(p => `<option value="${escapeAttr(p.id)}">${escapeHtml(p.name)} - ${Number(p.price || 0).toLocaleString('vi-VN')} đ</option>`).join('');
        $('#bc-product-id').html(options || '<option value="">Không có sản phẩm đang bán</option>');
      } catch (e) {
        alertBox('danger', `${I18N.alert.loadBroadcastProductsFailed}: ${e.message}`);
      }
    }

    let broadcastTemplates = [];

    async function loadBroadcastTemplates() {
      try {
        broadcastTemplates = await apiFetch('/broadcast/templates');
        renderBroadcastTemplateOptions();
        if (broadcastTemplates.length) {
          applyBroadcastTemplate(broadcastTemplates[0].id);
        }
      } catch (e) {
        alertBox('danger', `Tải mẫu thông báo thất bại: ${e.message}`);
      }
    }

    function renderBroadcastTemplateOptions() {
      const options = broadcastTemplates
        .map(t => `<option value="${escapeAttr(t.id)}">${escapeHtml(t.sort_order || t.id)}. ${escapeHtml(t.name)}</option>`)
        .join('');
      $('#bc-template-select').html(options);
    }

    function selectedBroadcastTemplate() {
      const id = Number($('#bc-template-select').val() || 0);
      return broadcastTemplates.find(t => Number(t.id) === id) || null;
    }

    function applyBroadcastTemplate(id) {
      const template = broadcastTemplates.find(t => Number(t.id) === Number(id));
      if (!template) return;
      $('#bc-template-select').val(String(template.id));
      $('#bc-template-name').val(template.name || '');
      $('#bc-text').val(template.text || '');
      $('#bc-mode').val(template.mode || 'message_only');
      $('#bc-buttons-json').val(template.buttons_json || '[]');
      if (template.product_id) {
        $('#bc-product-id').val(String(template.product_id));
      }
      toggleBroadcastProductPicker();
    }

    async function saveBroadcastTemplate() {
      const template = selectedBroadcastTemplate();
      if (!template) {
        alertBox('warning', 'Chọn mẫu trước khi lưu.');
        return;
      }
      const buttonsJson = ($('#bc-buttons-json').val() || '').trim() || '[]';
      try {
        JSON.parse(buttonsJson);
      } catch (_) {
        alertBox('danger', 'JSON nút không hợp lệ.');
        return;
      }
      const mode = $('#bc-mode').val() || 'message_only';
      const payload = {
        name: ($('#bc-template-name').val() || '').trim(),
        text: ($('#bc-text').val() || '').trim(),
        mode,
        buttons_json: buttonsJson,
        product_id: mode === 'new_product' ? Number($('#bc-product-id').val() || 0) || null : null,
        sort_order: Number(template.sort_order || template.id),
      };
      if (!payload.name || !payload.text) {
        alertBox('warning', 'Tên mẫu và nội dung không được trống.');
        return;
      }
      try {
        const updated = await apiFetch(`/broadcast/templates/${encodeURIComponent(template.id)}`, {
          method: 'PUT',
          body: JSON.stringify(payload),
        });
        const index = broadcastTemplates.findIndex(t => Number(t.id) === Number(updated.id));
        if (index >= 0) broadcastTemplates[index] = updated;
        renderBroadcastTemplateOptions();
        $('#bc-template-select').val(String(updated.id));
        alertBox('success', 'Đã lưu mẫu thông báo.');
      } catch (e) {
        alertBox('danger', `Lưu mẫu thất bại: ${e.message}`);
      }
    }

    function toggleBroadcastProductPicker() {
      const mode = $('#bc-mode').val() || 'message_only';
      const needsProduct = mode === 'new_product';
      $('#bc-product-wrap').toggleClass('d-none', !needsProduct);
      if (needsProduct && !$('#bc-product-id option').length) {
        loadBroadcastProducts();
      }
    }

    async function sendBroadcast() {
      const text = $('#bc-text').val().trim();
      if (!text) {
        alertBox('warning', I18N.alert.requiredBroadcastContent);
        return;
      }
      const mode = $('#bc-mode').val() || 'message_only';
      const productId = $('#bc-product-id').val();
      if (mode === 'new_product' && !productId) {
        alertBox('warning', I18N.alert.requiredBroadcastProduct);
        return;
      }
      const formData = new FormData();
      formData.append('text', text);
      formData.append('mode', mode);
      const buttonsJson = ($('#bc-buttons-json').val() || '').trim();
      if (buttonsJson) {
        try {
          JSON.parse(buttonsJson);
        } catch (_) {
          alertBox('danger', 'JSON nút không hợp lệ.');
          return;
        }
        formData.append('buttons_json', buttonsJson);
      }
      if (mode === 'new_product') {
        formData.append('product_id', productId);
      }
      const fileInput = document.getElementById('bc-image');
      if (fileInput.files && fileInput.files[0]) {
        formData.append('image', fileInput.files[0]);
      }
      try {
        const res = await fetch(API_BASE + '/broadcast', {
          method: 'POST',
          credentials: 'same-origin',
          body: formData,
        });
        if (!res.ok) {
          const data = await res.json().catch(() => null);
          const msg = data?.error?.message || res.statusText;
          throw new Error(msg);
        }
        alertBox('success', I18N.alert.broadcastQueued);
      } catch (e) {
        alertBox('danger', `${I18N.alert.broadcastFailed}: ${e.message}`);
      }
    }

    function debounce(fn, delay) {
      let timer = null;
      return function (...args) {
        const ctx = this;
        clearTimeout(timer);
        timer = setTimeout(() => fn.apply(ctx, args), delay);
      };
    }
    function setFilterLoading(selector, isLoading) {
      $(selector).toggleClass('d-none', !isLoading);
    }

    async function apiFetch(path, options = {}) {
      const normalizedPath = path.startsWith('/admin/') ? path.replace('/admin', '') : path;
      const headers = Object.assign({ 'Content-Type': 'application/json' }, options.headers || {});
      const res = await fetch(API_BASE + normalizedPath, Object.assign({}, options, { headers, credentials: 'same-origin' }));
      const data = await res.json().catch(() => null);
      if (!res.ok || (data && data.ok === false)) {
        const msg = data?.error?.message || res.statusText || I18N.alert.requestFailed;
        throw new Error(msg);
      }
      return data?.data ?? data;
    }

    function badgeStatus(status) {
      const map = { pending: 'warning', confirming: 'info', paid: 'success', failed: 'danger', cancel: 'secondary', expired: 'danger' };
      const cls = map[status] || 'light';
      return `<span class="badge bg-${cls} badge-status">${escapeHtml(status)}</span>`;
    }

