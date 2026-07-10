    function ensureProductUsageInstructionsField() {
      if ($('#product-usage-instructions').length) return;
      const html = `
            <div class="col-sm-12" id="product-usage-instructions-group">
              <label class="form-label">Hướng dẫn sử dụng sau mua</label>
              <textarea class="form-control" id="product-usage-instructions" rows="5"
                placeholder="Nội dung này sẽ tự gửi cho khách sau khi mua hàng thành công"></textarea>
              <small class="text-muted">Để trống nếu sản phẩm không cần gửi hướng dẫn riêng. Tối đa 4000 ký tự.</small>
            </div>`;
      $('#product-description').closest('.col-sm-12').after(html);
    }

    async function loadProductUsageInstructions(productId) {
      if (!productId) {
        $('#product-usage-instructions').val('');
        return;
      }
      try {
        const usage = await apiFetch(`/products/${productId}/usage-instructions`);
        $('#product-usage-instructions').val(usage?.content || '');
      } catch (e) {
        $('#product-usage-instructions').val('');
        console.warn('load product usage instructions failed', e);
      }
    }

    async function saveProductUsageInstructions(productId, content, options = {}) {
      if (!productId) return;
      const value = (content || '').trim();
      if (value) {
        await apiFetch(`/products/${productId}/usage-instructions`, {
          method: 'PUT',
          body: JSON.stringify({ content: value }),
        });
      } else if (options.allowDelete) {
        await apiFetch(`/products/${productId}/usage-instructions`, { method: 'DELETE' });
      }
    }

    function installProductUsageInstructionsHooks() {
      ensureProductUsageInstructionsField();

      if (typeof openProductModal === 'function' && !openProductModal.__usageInstructionsHooked) {
        const originalOpenProductModal = openProductModal;
        openProductModal = function (product = null) {
          ensureProductUsageInstructionsField();
          $('#product-usage-instructions').val('');
          const result = originalOpenProductModal.apply(this, arguments);
          if (product?.id) {
            loadProductUsageInstructions(product.id);
          }
          return result;
        };
        openProductModal.__usageInstructionsHooked = true;
      }

      if (typeof saveProduct === 'function' && !saveProduct.__usageInstructionsHooked) {
        const originalSaveProduct = saveProduct;
        saveProduct = async function () {
          ensureProductUsageInstructionsField();
          const existingId = $('#product-id').val();
          const usageContent = $('#product-usage-instructions').val();
          let savedProductId = existingId || null;
          const originalApiFetch = apiFetch;

          apiFetch = async function (url, options = {}) {
            const data = await originalApiFetch.apply(this, arguments);
            const method = (options.method || 'GET').toUpperCase();
            if (method === 'POST' && url === '/products' && data?.id) {
              savedProductId = data.id;
            } else if (method === 'PUT' && /^\/products\/\d+$/.test(url)) {
              savedProductId = url.split('/').pop();
            }
            return data;
          };

          try {
            const result = await originalSaveProduct.apply(this, arguments);
            if (savedProductId) {
              await saveProductUsageInstructions(savedProductId, usageContent, {
                allowDelete: !!existingId,
              });
            }
            return result;
          } finally {
            apiFetch = originalApiFetch;
          }
        };
        saveProduct.__usageInstructionsHooked = true;
      }
    }

    $(document).ready(function () {
      $('button').each(function () {
        const btn = $(this);
        const label = (btn.text() || '').trim();
        if (!btn.attr('aria-label') && (label === '↻' || label.length === 1)) {
          btn.attr('aria-label', 'Nút thao tác');
        }
      });
      installProductUsageInstructionsHooks();
      bindEvents();
      checkAuth();
    });
