    function applyProductFilters() {
      productOffset = 0;
      loadProducts();
    }

    function applyOrderFilters() {
      orderOffset = 0;
      loadOrders();
    }

    function applyWebhookFilters() {
      webhookOffset = 0;
      loadWebhookEvents();
    }

    async function reorderProductsInList(sourceId, targetId) {
      const sourceIdx = allLoadedProducts.findIndex(p => p.id === sourceId);
      const targetIdx = allLoadedProducts.findIndex(p => p.id === targetId);
      if (sourceIdx === -1 || targetIdx === -1) return;

      const [sourceProduct] = allLoadedProducts.splice(sourceIdx, 1);
      allLoadedProducts.splice(targetIdx, 0, sourceProduct);

      await saveNewProductOrders();
    }

    async function moveProductUpDown(id, direction) {
      const idx = allLoadedProducts.findIndex(p => p.id === id);
      if (idx === -1) return;

      if (direction === 'up' && idx > 0) {
        const [product] = allLoadedProducts.splice(idx, 1);
        allLoadedProducts.splice(idx - 1, 0, product);
      } else if (direction === 'down' && idx < allLoadedProducts.length - 1) {
        const [product] = allLoadedProducts.splice(idx, 1);
        allLoadedProducts.splice(idx + 1, 0, product);
      } else {
        return;
      }

      await saveNewProductOrders();
    }

    async function saveNewProductOrders() {
      const items = allLoadedProducts.map((p, index) => ({
        id: p.id,
        sort_order: index + 1
      }));

      try {
        await apiFetch('/products/reorder', {
          method: 'POST',
          body: JSON.stringify({ items }),
        });
        loadProducts();
      } catch (e) {
        alertBox('danger', 'Cập nhật thứ tự thất bại: ' + e.message);
      }
    }

    async function loadProductCategories() {
      try {
        productCategories = await apiFetch('/product-categories') || [];
        renderProductCategories();
        renderProductCategoryOptions();
      } catch (e) {
        alertBox('danger', `${I18N.alert.loadCategoriesFailed}: ${e.message}`);
      }
    }

    function categoryEmojiLabel(category) {
      if (!category) return '';
      const emoji = (category.emoji || '').trim();
      const customId = (category.custom_emoji_id || '').trim();
      if (customId) return `{${customId}}`;
      return emoji;
    }

    function renderProductCategories() {
      const rows = (productCategories || []).map(category => {
        const emoji = category.emoji ? escapeHtml(category.emoji) : '<span class="text-muted">-</span>';
        const customId = category.custom_emoji_id ? `<code>${escapeHtml(category.custom_emoji_id)}</code>` : '<span class="text-muted">-</span>';
        return `
          <tr>
            <td>
              <span class="fw-bold">${escapeHtml(category.name)}</span>
              ${categoryEmojiLabel(category) ? `<div class="small text-muted">Menu: ${escapeHtml(categoryEmojiLabel(category))}${escapeHtml(category.name)}</div>` : ''}
            </td>
            <td>${emoji}</td>
            <td>${customId}</td>
            <td>${category.sort_order ?? category.id}</td>
            <td>
              <div class="btn-group btn-group-sm">
                <button class="btn btn-outline-secondary edit-product-category" data-id="${category.id}">Sửa</button>
                <button class="btn btn-outline-danger delete-product-category" data-id="${category.id}">Ẩn</button>
              </div>
            </td>
          </tr>
        `;
      }).join('');
      $('#product-categories-body').html(rows || `<tr><td colspan="5" class="text-center text-muted">${I18N.common.empty}</td></tr>`);
    }

    function renderProductCategoryOptions(selectedId = '', fallbackName = '') {
      const selected = selectedId ? String(selectedId) : '';
      const options = [
        '<option value="">Chưa chọn</option>',
        ...(productCategories || []).map(category => {
          const icon = categoryEmojiLabel(category);
          const label = `${icon ? `${icon} ` : ''}${category.name}`;
          return `<option value="${category.id}" ${String(category.id) === selected ? 'selected' : ''}>${escapeHtml(label)}</option>`;
        }),
      ];
      if (!selected && fallbackName) {
        options.push(`<option value="" selected>${escapeHtml(fallbackName)}</option>`);
      }
      $('#product-category-id').html(options.join('')).data('legacy-category', fallbackName || '');
    }

    async function loadProducts() {
      const activeVal = $('#product-active').val();
      const active = activeVal === 'all' ? '' : `&active=${activeVal}`;
      const q = $('#product-query').val().trim();
      const query = q ? `&query=${encodeURIComponent(q)}` : '';
      setFilterLoading('#products-filter-loading', true);
      try {
        await loadProductCategories();
        const data = await apiFetch(`/products?limit=20&offset=${productOffset}${active}${query}`);
        allLoadedProducts = data.items || [];
        const rows = allLoadedProducts.map(p => {
          const productName = escapeHtml(p.name);
          const productNameAttr = escapeAttr(p.name);
          const categoryIcon = p.category_custom_emoji_id ? `{${p.category_custom_emoji_id}}` : (p.category_emoji || '');
          const categoryLabel = p.category ? `<small class="text-muted">${escapeHtml(categoryIcon ? `${categoryIcon} ${p.category}` : p.category)}</small>` : '';
            const buttonEmojiLabel = p.button_emoji ? `<small class="text-muted">Nút: ${escapeHtml(p.button_emoji)}</small>` : '';
            const buttonCustomEmojiLabel = p.button_custom_emoji_id ? `<small class="text-muted">Emoji động: ${escapeHtml(p.button_custom_emoji_id)}</small>` : '';
          const deliveryType = productDeliveryType(p);
          const isManual = deliveryType === 'manual_input';
          const isUploadedFile = deliveryType === 'uploaded_file';
          const isStock = deliveryType === 'stock_item';
          const typeLabel = deliveryType === 'manual_input'
            ? '<span class="badge bg-info">Nhập liệu</span>'
            : deliveryType === 'uploaded_file'
              ? '<span class="badge bg-primary">File</span>'
              : '<span class="badge bg-secondary">Kho item</span>';
          const stockCell = isUploadedFile
            ? (p.stock_count > 0
              ? `<span class="badge bg-success">${p.stock_count} file</span>`
              : (p.file_name ? `<span class="badge bg-success">File cũ</span><div class="small text-muted">${escapeHtml(p.file_name)}</div>` : '<span class="badge bg-warning text-dark">Chưa có file</span>'))
            : isManual
              ? '<span class="text-muted">Theo gói</span>'
              : p.stock_count;
          const plansBtn = isManual
            ? `<button class="btn btn-sm btn-outline-warning manage-plans" data-id="${p.id}" data-name="${productNameAttr}">Gói</button>`
            : '';
          const addItemsBtn = isStock
            ? `<button class="btn btn-sm btn-outline-success add-items" data-id="${p.id}">Thêm item</button>`
            : '';
          const manageItemsRow = isStock
            ? `<li><a class="dropdown-item manage-items" href="#" data-id="${p.id}" data-name="${productNameAttr}">Kho Items</a></li>
             <li><a class="dropdown-item add-items" href="#" data-id="${p.id}">Thêm Items</a></li>`
            : isUploadedFile
              ? `<li><a class="dropdown-item manage-items" href="#" data-id="${p.id}" data-name="${productNameAttr}">Kho file</a></li>`
              : '';
          const managePlansRow = isManual
            ? `<li><a class="dropdown-item manage-plans" href="#" data-id="${p.id}" data-name="${productNameAttr}">Quản lý Gói</a></li>`
            : '';
          const imgTag = p.image_url ? `<img src="${escapeAttr(p.image_url)}" style="width:40px; height:40px; object-fit:cover; border-radius:4px;" class="me-2" alt="img">` : '';
          return `
        <tr class="product-row" draggable="true" data-id="${p.id}">
          <td class="mobile-priority">
            <div class="d-flex align-items-center">
              ${imgTag}
              <div class="d-flex flex-column">
                <span class="fw-bold text-dark">${productName}</span>
                 ${categoryLabel}
                 ${buttonEmojiLabel}
                 ${buttonCustomEmojiLabel}
                 <small class="text-muted d-md-none">ID: ${p.id}</small>
              </div>
            </div>
          </td>
          <td class="mobile-priority"><span class="fw-bold text-primary">${p.price.toLocaleString('vi-VN')}</span></td>
          <td class="d-none d-md-table-cell mobile-hide">${stockCell}</td>
          <td class="d-none d-md-table-cell mobile-hide">${typeLabel}</td>
          <td class="d-none d-md-table-cell mobile-hide">
            <div class="form-check form-switch">
              <input class="form-check-input product-toggle" data-id="${p.id}" type="checkbox" ${p.is_active === 1 ? 'checked' : ''}>
            </div>
          </td>
          <td class="mobile-priority">
            <div class="d-flex gap-1 align-items-center">
              <button class="btn btn-xs btn-outline-secondary btn-move-up p-1 touch-target border-0" data-id="${p.id}" style="font-size: 12px; line-height: 1;" title="Di chuyển lên">▲</button>
              <button class="btn btn-xs btn-outline-secondary btn-move-down p-1 touch-target border-0" data-id="${p.id}" style="font-size: 12px; line-height: 1;" title="Di chuyển xuống">▼</button>
              <span class="text-muted d-none d-md-inline-block ms-1" style="font-size: 12px; cursor: grab;" title="Kéo thả để sắp xếp">☰</span>
            </div>
          </td>
          <td class="d-none d-lg-table-cell mobile-hide small text-muted">${escapeHtml(p.created_at || '')}</td>
          <td class="mobile-priority">
            <div class="dropdown">
              <button class="btn btn-sm btn-light border dropdown-toggle touch-target" type="button" data-bs-toggle="dropdown" aria-label="Tùy chọn chỉnh sửa sản phẩm">Sửa</button>
              <ul class="dropdown-menu shadow-sm border-0">
                <li><a class="dropdown-item edit-product" href="#" data-id="${p.id}">Chỉnh sửa</a></li>
                ${manageItemsRow}
                ${managePlansRow}
                <li><hr class="dropdown-divider"></li>
                <li><a class="dropdown-item delete-product text-danger" href="#" data-id="${p.id}">Ngưng bán</a></li>
              </ul>
            </div>
          </td>
        </tr>
      `;
        }).join('');
        $('#products-body').html(rows || `<tr><td colspan="10" class="text-center text-muted">${I18N.common.empty}</td></tr>`);
      } catch (e) {
        alertBox('danger', `${I18N.alert.loadProductsFailed}: ${e.message}`);
      } finally {
        setFilterLoading('#products-filter-loading', false);
      }
    }


    function productDeliveryType(product) {
      if (product?.delivery_type) return product.delivery_type;
      return product?.requires_input === 1 ? 'manual_input' : 'stock_item';
    }

    function openProductModal(product = null) {
      $('#product-id').val(product?.id || '');
      $('#product-name').val(product?.name || '');
      $('#product-price').val(product?.price ?? 0);
      $('#product-active-input').val(product?.is_active ?? 1);
        $('#product-delivery-type').val(productDeliveryType(product));
        renderProductCategoryOptions(product?.category_id || '', product?.category || '');
        $('#product-button-emoji').val(product?.button_emoji || '');
        $('#product-button-custom-emoji-id').val(product?.button_custom_emoji_id || '');
        $('#product-input-prompt').val(product?.input_prompt || '');
      $('#product-description').val(product?.description || '');
      $('#product-show-sold-count').prop('checked', Number(product?.show_sold_count || 0) === 1);
      $('#product-image').val('');
      if (product?.image_url) {
        $('#product-image-preview').attr('src', product.image_url);
        $('#product-image-current').removeClass('d-none').addClass('d-flex');
        $('#delete-product-image-btn').data('id', product.id);
      } else {
        $('#product-image-preview').attr('src', '');
        $('#product-image-current').addClass('d-none').removeClass('d-flex');
        $('#delete-product-image-btn').removeData('id');
      }
      $('#product-file').val('');
      $('#product-file-current').text(product?.stock_count > 0 ? `Kho hiện có ${product.stock_count} file.` : (product?.file_name ? `File cũ: ${product.file_name}` : 'Chưa có file trong kho.'));
      $('#productModalLabel').text(product ? 'Chỉnh sửa sản phẩm' : 'Sản phẩm mới');
      toggleProductDeliveryInputs();
      new bootstrap.Modal(document.getElementById('productModal')).show();
    }

    function toggleProductDeliveryInputs() {
      const deliveryType = $('#product-delivery-type').val();
      const priceInput = $('#product-price');
      if (deliveryType === 'manual_input') {
        priceInput.val(0).prop('disabled', true);
      } else {
        priceInput.prop('disabled', false);
      }
      $('#product-input-prompt-group').toggleClass('d-none', deliveryType !== 'manual_input');
      $('#product-file-group').toggleClass('d-none', deliveryType !== 'uploaded_file');
    }

    async function saveProduct() {
      const id = $('#product-id').val();
      const deliveryType = $('#product-delivery-type').val();
      const requiresInput = deliveryType === 'manual_input' ? 1 : 0;
      const priceVal = deliveryType === 'manual_input' ? 0 : Number($('#product-price').val());
      const payload = {
        name: $('#product-name').val().trim(),
        price: priceVal,
        is_active: Number($('#product-active-input').val()),
        requires_input: requiresInput,
         delivery_type: deliveryType,
         category_id: Number($('#product-category-id').val()) || null,
         category: Number($('#product-category-id').val()) ? null : ($('#product-category-id').data('legacy-category') || null),
         button_emoji: $('#product-button-emoji').val().trim() || null,
         button_custom_emoji_id: $('#product-button-custom-emoji-id').val().trim() || null,
         input_prompt: $('#product-input-prompt').val().trim() || null,
        description: $('#product-description').val().trim() || null,
        show_sold_count: $('#product-show-sold-count').is(':checked') ? 1 : 0,
      };
      const method = id ? 'PUT' : 'POST';
      const url = id ? `/products/${id}` : '/products';
      try {
        const resData = await apiFetch(url, { method, body: JSON.stringify(payload) });
        const productId = id || resData.id;

        const fileInput = document.getElementById('product-image');
        let imageUploadFailed = false;
        if (fileInput.files && fileInput.files[0]) {
          try {
            const formData = new FormData();
            formData.append('image', fileInput.files[0]);
            const uploadRes = await fetch(`${API_BASE}/products/${productId}/image`, {
              method: 'POST',
              credentials: 'same-origin',
              body: formData,
            });
            if (!uploadRes.ok) {
              imageUploadFailed = true;
            }
          } catch (_) {
            imageUploadFailed = true;
          }
        }

        const deliveryFileInput = document.getElementById('product-file');
        let deliveryUploadFailed = false;
        if (deliveryType === 'uploaded_file' && deliveryFileInput.files && deliveryFileInput.files[0]) {
          try {
            const formData = new FormData();
            for (const file of deliveryFileInput.files) {
              formData.append('file', file);
            }
            const uploadRes = await fetch(`${API_BASE}/products/${productId}/file`, {
              method: 'POST',
              credentials: 'same-origin',
              body: formData,
            });
            if (!uploadRes.ok) {
              deliveryUploadFailed = true;
            }
          } catch (_) {
            deliveryUploadFailed = true;
          }
        }

        bootstrap.Modal.getInstance(document.getElementById('productModal')).hide();
        if (imageUploadFailed) {
          alertBox('warning', I18N.alert.uploadImageFailed);
        } else if (deliveryUploadFailed) {
          alertBox('warning', I18N.alert.uploadFileFailed);
        } else {
          alertBox('success', I18N.alert.savedProduct);
        }
        loadProducts();
      } catch (e) {
        alertBox('danger', `${I18N.alert.saveFailed}: ${e.message}`);
      }
    }

    function openProductCategoryModal(category = null) {
      showInputActionModal({
        title: category ? 'Sửa danh mục' : 'Danh mục mới',
        description: 'Emoji động dùng ID số của custom emoji Telegram. Khi có ID động, hệ thống ưu tiên ID động thay cho emoji thường.',
        confirmText: category ? 'Cập nhật' : 'Thêm danh mục',
        fields: [
          { name: 'name', label: 'Tên danh mục', value: category?.name || '', required: true, maxlength: 64, placeholder: 'VD: CAP CUT' },
          { name: 'emoji', label: 'Emoji thường', value: category?.emoji || '', maxlength: 16, placeholder: 'VD: ✨' },
          { name: 'custom_emoji_id', label: 'Custom emoji ID động', value: category?.custom_emoji_id || '', maxlength: 32, placeholder: 'VD: 5375135722514685501' },
          { name: 'sort_order', label: 'Sắp xếp', value: category?.sort_order ?? '', type: 'number', placeholder: 'Nhỏ hiện trước' },
        ],
        onSubmit: async (values) => {
          const payload = {
            name: values.name.trim(),
            emoji: values.emoji.trim() || null,
            custom_emoji_id: values.custom_emoji_id.trim() || null,
            sort_order: values.sort_order ? Number(values.sort_order) : null,
            is_active: 1,
          };
          const method = category ? 'PUT' : 'POST';
          const url = category ? `/product-categories/${category.id}` : '/product-categories';
          await apiFetch(url, { method, body: JSON.stringify(payload) });
          alertBox('success', I18N.alert.savedCategory);
          await loadProductCategories();
          await loadProducts();
        },
      });
    }

    function deleteProductCategory(id) {
      showConfirmActionModal({
        title: 'Ẩn danh mục',
        description: 'Danh mục sẽ không còn hiện trong danh sách chọn. Sản phẩm cũ vẫn giữ tên danh mục đã lưu.',
        confirmText: 'Ẩn danh mục',
        confirmClass: 'btn-danger',
        onConfirm: async () => {
          await apiFetch(`/product-categories/${id}`, { method: 'DELETE' });
          alertBox('success', I18N.alert.deletedCategory);
          await loadProductCategories();
          await loadProducts();
        },
      });
    }

    async function deleteCurrentProductImage() {
      const id = $('#delete-product-image-btn').data('id') || $('#product-id').val();
      if (!id) return;

      showConfirmActionModal({
        title: 'Xóa ảnh sản phẩm',
        description: 'Ảnh hiện tại sẽ bị gỡ khỏi sản phẩm.',
        context: [`Mã sản phẩm: ${id}`],
        confirmText: 'Xóa ảnh',
        confirmClass: 'btn-danger',
        onConfirm: async () => {
          try {
            await apiFetch(`/products/${id}/image`, { method: 'DELETE' });
            $('#product-image').val('');
            $('#product-image-preview').attr('src', '');
            $('#product-image-current').addClass('d-none').removeClass('d-flex');
            $('#delete-product-image-btn').removeData('id');
            alertBox('success', I18N.alert.deletedProductImage);
            await loadProducts();
          } catch (e) {
            alertBox('danger', `${I18N.alert.deleteProductImageFailed}: ${e.message}`);
          }
        },
      });
    }

    async function fetchProduct(id) {
      return apiFetch(`/products/${id}`);
    }

    function openItemsModal(productId) {
      $('#item-product-id').val(productId);
      $('#item-textarea').val('');
      new bootstrap.Modal(document.getElementById('itemModal')).show();
    }

    async function saveItems() {
      const productId = $('#item-product-id').val();
      const lines = $('#item-textarea').val().split(/\r?\n/).map(s => s.trim()).filter(Boolean);
      if (!lines.length) {
        alertBox('warning', I18N.alert.needOneItem);
        return;
      }
      try {
        await apiFetch(`/products/${productId}/items`, { method: 'POST', body: JSON.stringify({ items: lines }) });
        bootstrap.Modal.getInstance(document.getElementById('itemModal')).hide();
        alertBox('success', I18N.alert.addItemsSuccess.replace('{count}', lines.length));
        loadProducts();
        if (currentItemProduct && currentItemProduct.id === Number(productId)) {
          loadItemsList();
        }
      } catch (e) {
        alertBox('danger', `${I18N.alert.addItemsFailed}: ${e.message}`);
      }
    }

    async function loadItemsList() {
      if (!currentItemProduct) return;
      try {
        const data = await apiFetch(`/products/${currentItemProduct.id}/items?limit=20&offset=${itemsOffset}`);
        $('#items-total').text(data.total);
        const rows = data.items.map(it => `
        <tr>
          <td>${it.id}</td>
          <td><code>${escapeHtml(it.content)}</code></td>
          <td>${escapeHtml(it.created_at || '')}</td>
          <td class="text-end d-flex gap-2 justify-content-end">
            <button class="btn btn-sm btn-outline-primary copy-item" data-content="${escapeAttr(it.content)}">Sao chép</button>
            <button class="btn btn-sm btn-outline-danger delete-item" data-id="${it.id}">Xóa</button>
          </td>
        </tr>
      `).join('');
        $('#items-body').html(rows || '<tr><td colspan="4" class="text-center text-muted">Không có item</td></tr>');
        $('#items-prev').prop('disabled', itemsOffset <= 0);
        $('#items-next').prop('disabled', itemsOffset + 20 >= data.total);
      } catch (e) {
        alertBox('danger', 'Tải danh sách item thất bại: ' + e.message);
      }
    }

    function openItemsList(productId, name) {
      currentItemProduct = { id: Number(productId), name };
      itemsOffset = 0;
      $('#items-list-title').text(`Item của ${name}`);
      $('#items-list-subtitle').text(`Mã sản phẩm: ${productId}`);
      loadItemsList();
      new bootstrap.Modal(document.getElementById('itemsListModal')).show();
    }

    async function deleteItem(itemId) {
      if (!currentItemProduct) return;
      showConfirmActionModal({
        title: 'Xóa item sản phẩm',
        description: 'Bạn có chắc muốn xóa item này?',
        context: [`Mã sản phẩm: ${currentItemProduct.id}`, `Mã item: ${itemId}`],
        confirmText: 'Xóa item',
        confirmClass: 'btn-danger',
        onConfirm: async () => {
          await apiFetch(`/products/${currentItemProduct.id}/items/${itemId}`, { method: 'DELETE' });
          alertBox('success', I18N.alert.deleteItemSuccess);
          await loadItemsList();
          await loadProducts();
        },
      });
    }

    // -------- Plans (pricing options) ----------
    function resetPlanForm() {
      editingPlanId = null;
      $('#plan-label').val('');
      $('#plan-months').val(1);
      $('#plan-price').val(0);
      $('#plan-sort').val(0);
    }

    async function loadPlans() {
      if (!currentPlanProduct) return;
      try {
        const plans = await apiFetch(`/products/${currentPlanProduct.id}/plans`);
        const rows = plans.map(pl => `
        <tr>
          <td>${pl.id}</td>
          <td>${escapeHtml(pl.label)}</td>
          <td>${pl.months}</td>
          <td>${pl.price.toLocaleString('vi-VN')} ₫</td>
          <td>${escapeHtml(pl.sort_order ?? '')}</td>
          <td class="d-flex gap-2">
            <button class="btn btn-sm btn-outline-primary edit-plan" 
              data-id="${pl.id}" 
              data-label="${escapeAttr(pl.label)}" 
              data-months="${pl.months}" 
              data-price="${pl.price}" 
              data-sort="${pl.sort_order ?? 0}">Sửa</button>
            <button class="btn btn-sm btn-outline-danger delete-plan" data-id="${pl.id}">Xóa</button>
          </td>
        </tr>
      `).join('');
        $('#plans-body').html(rows || '<tr><td colspan="6" class="text-center text-muted">Không có gói</td></tr>');
      } catch (e) {
        alertBox('danger', 'Tải danh sách gói thất bại: ' + e.message);
      }
    }

    function openPlans(productId, name) {
      currentPlanProduct = { id: Number(productId), name };
      $('#plans-product-id').val(productId);
      $('#plans-title').text(`Gói của ${name}`);
      $('#plans-subtitle').text(`Mã sản phẩm: ${productId}`);
      resetPlanForm();
      loadPlans();
      new bootstrap.Modal(document.getElementById('plansModal')).show();
    }

    async function savePlan() {
      if (!currentPlanProduct) return;
      const payload = {
        label: $('#plan-label').val().trim(),
        months: Number($('#plan-months').val()),
        price: Number($('#plan-price').val()),
        sort_order: Number($('#plan-sort').val()) || 0,
      };
      const isEdit = !!editingPlanId;
      const url = isEdit ? `/products/${currentPlanProduct.id}/plans/${editingPlanId}` : `/products/${currentPlanProduct.id}/plans`;
      const method = isEdit ? 'PUT' : 'POST';
      try {
        await apiFetch(url, { method, body: JSON.stringify(payload) });
        alertBox('success', isEdit ? I18N.alert.planUpdated : I18N.alert.planAdded);
        resetPlanForm();
        loadPlans();
      } catch (e) {
        alertBox('danger', `${I18N.alert.savePlanFailed}: ${e.message}`);
      }
    }

    async function deletePlan(id) {
      showConfirmActionModal({
        title: 'Xóa gói giá',
        description: 'Bạn có chắc muốn xóa gói này?',
        context: [
          currentPlanProduct ? `Mã sản phẩm: ${currentPlanProduct.id}` : 'Mã sản phẩm: -',
          `Mã gói: ${id}`,
        ],
        confirmText: 'Xóa gói',
        confirmClass: 'btn-danger',
        onConfirm: async () => {
          await apiFetch(`/products/${currentPlanProduct.id}/plans/${id}`, { method: 'DELETE' });
          alertBox('success', I18N.alert.deletePlanSuccess);
          await loadPlans();
        },
      });
    }

    async function toggleProduct(id, active) {
      try {
        await apiFetch(`/products/${id}/toggle`, { method: 'POST', body: JSON.stringify({ is_active: active }) });
        alertBox('success', I18N.alert.updatedStatus);
        loadProducts();
      } catch (e) {
        alertBox('danger', `${I18N.alert.toggleFailed}: ${e.message}`);
        loadProducts();
      }
    }


    async function deleteProduct(id) {
      showConfirmActionModal({
        title: 'Ngưng bán sản phẩm',
        description: 'Thao tác này sẽ disable sản phẩm.',
        context: [`Mã sản phẩm: ${id}`],
        confirmText: 'Ngưng bán',
        confirmClass: 'btn-danger',
        onConfirm: async () => {
          await apiFetch(`/products/${id}`, { method: 'DELETE' });
          alertBox('success', I18N.alert.disabledProduct);
          await loadProducts();
        },
      });
    }

