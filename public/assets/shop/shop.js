(() => {
  "use strict";

  const state = {
    products: [],
    filteredProducts: [],
    categories: [],
    category: "Tất cả",
    customer: null,
    authMode: "login",
    selectedProduct: null,
    selectedPlan: null,
    quantity: 1,
    currentOrder: null,
    pollTimer: null,
    checkoutAfterAuth: false,
    usingFallback: false,
  };

  const $ = (selector, root = document) => root.querySelector(selector);
  const $$ = (selector, root = document) => [...root.querySelectorAll(selector)];
  const money = value => `${new Intl.NumberFormat("vi-VN").format(Number(value) || 0)}đ`;
  const escapeHtml = value => String(value ?? "").replace(/[&<>'"]/g, char => ({
    "&": "&amp;",
    "<": "&lt;",
    ">": "&gt;",
    "'": "&#39;",
    '"': "&quot;",
  }[char]));
  const iconRefresh = () => window.lucide?.createIcons?.();

  const fallbackProducts = [
    { id: 101, name: "Netflix Premium", price: 79000, description: "Tài khoản xem phim chất lượng cao, thông tin giao ngay sau khi thanh toán.", image_url: null, category: "Streaming", category_emoji: null, delivery_type: "stock_item", requires_input: false, input_prompt: null, stock_count: 8, plans: [] },
    { id: 102, name: "Netflix Extra Member", price: 99000, description: "Gói thành viên phụ theo thời hạn, phù hợp cho thiết bị cá nhân.", image_url: null, category: "Streaming", category_emoji: null, delivery_type: "stock_item", requires_input: false, input_prompt: null, stock_count: 5, plans: [{ id: 1002, label: "1 tháng", months: 1, price: 99000 }, { id: 1003, label: "3 tháng", months: 3, price: 249000 }] },
    { id: 103, name: "Spotify Premium", price: 69000, description: "Nghe nhạc không quảng cáo, giao tài khoản tự động và riêng tư.", image_url: null, category: "Streaming", category_emoji: null, delivery_type: "stock_item", requires_input: false, input_prompt: null, stock_count: 12, plans: [] },
    { id: 104, name: "Tài khoản AI Pro", price: 129000, description: "Tài khoản dịch vụ số theo tháng, có hướng dẫn sử dụng sau khi mua.", image_url: null, category: "Dịch vụ số", category_emoji: null, delivery_type: "manual_input", requires_input: true, input_prompt: "Nhập email bạn muốn kích hoạt", stock_count: null, plans: [{ id: 1004, label: "1 tháng", months: 1, price: 129000 }, { id: 1005, label: "6 tháng", months: 6, price: 599000 }] },
  ];

  async function api(path, options = {}) {
    const response = await fetch(path, {
      credentials: "same-origin",
      headers: { "Content-Type": "application/json", ...(options.headers || {}) },
      ...options,
    });
    const body = await response.json().catch(() => ({}));
    if (!response.ok || body.ok === false) {
      throw new Error(body.error?.message || "Có lỗi xảy ra, vui lòng thử lại.");
    }
    return body.data;
  }

  function showToast(message, error = false) {
    const stack = $("[data-toast-stack]");
    if (!stack) return;
    const item = document.createElement("div");
    item.className = `toast${error ? " is-error" : ""}`;
    item.textContent = message;
    stack.append(item);
    window.setTimeout(() => item.remove(), 4200);
  }

  function setModal(name) {
    const backdrop = $("[data-modal-backdrop]");
    $$('[data-modal]', backdrop).forEach(modal => { modal.hidden = modal.dataset.modal !== name; });
    backdrop.hidden = !name;
    document.body.classList.toggle("modal-open", Boolean(name));
    iconRefresh();
  }

  function updateClock() {
    const target = $("[data-sync-time]");
    if (target) target.textContent = new Date().toLocaleTimeString("vi-VN", { hour12: false });
  }

  function renderCategories() {
    state.categories = ["Tất cả", ...new Set(state.products.map(product => product.category).filter(Boolean))];
    $("[data-category-tabs]").innerHTML = state.categories.map(category => `<button class="category-tab${category === state.category ? " is-active" : ""}" type="button" data-category="${escapeHtml(category)}">${escapeHtml(category)}</button>`).join("");
  }

  function filterProducts() {
    const query = $("[data-search]").value.trim().toLowerCase();
    state.filteredProducts = state.products.filter(product => {
      const matchesCategory = state.category === "Tất cả" || product.category === state.category;
      const haystack = `${product.name} ${product.description || ""} ${product.category || ""}`.toLowerCase();
      return matchesCategory && (!query || haystack.includes(query));
    });
    renderProducts();
  }

  function productArt(product) {
    if (product.image_url) return `<img src="${escapeHtml(product.image_url)}" alt="${escapeHtml(product.name)}" loading="lazy">`;
    const label = product.name.toUpperCase().includes("NETFLIX") ? "NETFLIX" : product.name.split(" ").slice(0, 2).join(" ").toUpperCase();
    return `<div class="product-art-fallback"><span>${escapeHtml(label)}</span></div>`;
  }

  function stockInfo(product) {
    if (product.stock_count === null || product.stock_count === undefined) return { text: "Theo yêu cầu", className: "" };
    if (product.stock_count < 1) return { text: "Hết hàng", className: " is-empty", disabled: true };
    if (product.stock_count < 4) return { text: `${product.stock_count} còn lại`, className: " is-low" };
    return { text: `${product.stock_count} sẵn hàng`, className: "" };
  }

  function renderProducts() {
    const status = $("[data-catalog-status]");
    const count = state.filteredProducts.length;
    status.textContent = state.usingFallback ? `${count} sản phẩm xem trước` : `${count} sản phẩm đang mở bán`;
    $("[data-product-count]").textContent = String(state.products.length).padStart(2, "0");
    $("[data-header-product-count]").textContent = `${state.products.length} sản phẩm`;
    $("[data-live-product-count]").textContent = state.products.length;

    const grid = $("[data-product-grid]");
    if (!count) {
      grid.innerHTML = `<div class="empty-state" style="grid-column: 1 / -1"><i data-lucide="search-x" aria-hidden="true"></i><h3>Không tìm thấy sản phẩm</h3><p>Thử tìm theo tên khác hoặc chọn lại danh mục trong kho.</p></div>`;
      iconRefresh();
      return;
    }

    grid.innerHTML = state.filteredProducts.map((product, index) => {
      const stock = stockInfo(product);
      const delivery = product.delivery_type === "manual_input" ? "Kích hoạt" : "Tự động";
      return `<article class="product-card${stock.disabled ? " is-disabled" : ""}" data-reveal style="--card-delay: ${Math.min(index, 7) * 45}ms"><div class="product-art">${productArt(product)}<span class="product-number">0${String(index + 1).padStart(1, "0")}</span><span class="stock-chip${stock.className}">${escapeHtml(stock.text)}</span></div><div class="product-body"><div class="product-meta"><span>${escapeHtml(product.category || "Dịch vụ số")}</span><span>${delivery}</span></div><h3>${escapeHtml(product.name)}</h3><p>${escapeHtml(product.description || "Thông tin sản phẩm sẽ được giao tự động sau khi hệ thống xác nhận thanh toán.")}</p><div class="product-footer"><span class="price">${money(product.price)} <small>/ từ</small></span><button class="mini-button" type="button" data-product-id="${product.id}" ${stock.disabled ? "disabled" : ""}>Xem chi tiết <i data-lucide="arrow-up-right" aria-hidden="true"></i></button></div></div></article>`;
    }).join("");
    iconRefresh();
    observeReveals(grid);
  }

  function openProduct(id) {
    state.selectedProduct = state.products.find(product => product.id === id);
    if (!state.selectedProduct) return;
    state.selectedPlan = state.selectedProduct.plans?.[0] || null;
    state.quantity = 1;
    renderProductDetail();
    setModal("product");
  }

  function renderProductDetail() {
    const product = state.selectedProduct;
    if (!product) return;
    const plan = state.selectedPlan;
    const amount = plan ? plan.price : product.price * state.quantity;
    const plans = product.plans?.length
      ? `<span class="field-label">Chọn gói thời hạn</span><div class="plan-options">${product.plans.map(item => `<button class="plan-option${state.selectedPlan?.id === item.id ? " is-active" : ""}" type="button" data-plan-id="${item.id}"><strong>${escapeHtml(item.label)}</strong><span>${money(item.price)}</span></button>`).join("")}</div>`
      : "";
    const input = product.requires_input
      ? `<label class="field-label" for="customer-input">${escapeHtml(product.input_prompt || "Thông tin cần kích hoạt")}</label><textarea class="detail-input" id="customer-input" data-customer-input required placeholder="Nhập chính xác thông tin cần dùng"></textarea>`
      : "";
    const quantity = product.plans?.length && product.delivery_type === "manual_input"
      ? ""
      : `<div class="qty-row"><span class="field-label">Số lượng</span><div class="qty-control"><button type="button" data-qty="minus" aria-label="Giảm số lượng">-</button><output data-qty-output>${state.quantity}</output><button type="button" data-qty="plus" aria-label="Tăng số lượng">+</button></div></div>`;
    $("[data-product-detail]").innerHTML = `<p class="detail-kicker">${escapeHtml(product.category || "DỊCH VỤ SỐ")} / CHI TIẾT</p><div class="product-detail-head"><div class="detail-image">${productArt(product)}</div><div><h2 class="detail-title" id="product-title">${escapeHtml(product.name)}</h2><p class="detail-description">${escapeHtml(product.description || "Sản phẩm được giao sau khi hệ thống xác nhận thanh toán.")}</p></div></div>${plans}${quantity}${input}<div class="detail-total"><span>Tổng thanh toán</span><strong>${money(amount)}</strong></div><button class="primary-button detail-buy" type="button" data-detail-buy><span>Mua ngay</span><i data-lucide="arrow-up-right" aria-hidden="true"></i></button>`;
    iconRefresh();
  }

  async function submitOrder() {
    if (!state.customer) {
      state.checkoutAfterAuth = true;
      setModal("auth");
      return;
    }
    const product = state.selectedProduct;
    const customerInput = $("[data-customer-input]")?.value.trim() || null;
    try {
      const result = await api("/api/shop/orders", {
        method: "POST",
        body: JSON.stringify({ product_id: product.id, quantity: state.quantity, plan_id: state.selectedPlan?.id || null, customer_input: customerInput }),
      });
      state.currentOrder = result.order;
      setModal("payment");
      renderPayment(result);
      loadProducts();
      loadOrders();
    } catch (error) {
      showToast(error.message, true);
    }
  }

  function renderPayment(result) {
    const order = result.order;
    const payment = result.payment;
    const paid = order.status === "paid";
    const pending = order.status === "pending";
    const delivery = paid ? renderDelivery(order.delivered_data) : "";
    const statusText = paid ? "Đã thanh toán" : pending ? "Đang chờ thanh toán" : order.status === "expired" ? "Đơn đã hết hạn" : "Đơn đã đóng";
    const statusClass = paid ? " is-paid" : pending ? "" : " is-closed";
    const paymentContent = pending && payment
      ? `<div class="payment-qr"><img src="${escapeHtml(payment.qr_url)}" alt="Mã QR thanh toán"></div><div class="payment-info"><div class="payment-line"><span>Ngân hàng</span><strong>${escapeHtml(payment.bank_name)}</strong></div><div class="payment-line"><span>Số tài khoản</span><strong>${escapeHtml(payment.account)}</strong></div><div class="payment-line"><span>Chủ tài khoản</span><strong>${escapeHtml(payment.account_name || "")}</strong></div><div class="payment-line"><span>Số tiền</span><strong class="highlight">${money(payment.amount)}</strong></div><div class="payment-line"><span>Nội dung</span><strong class="highlight">${escapeHtml(payment.memo)}</strong></div></div><p class="payment-note"><strong>Quan trọng:</strong> chuyển đúng số tiền và nội dung trên. Cửa sổ này tự cập nhật khi hệ thống nhận được giao dịch.</p>`
      : paid
        ? delivery
        : `<div class="delivery-box"><h3>Đơn không còn hiệu lực</h3><p class="payment-note">Đơn này không còn nhận thanh toán. Bạn có thể tạo một đơn mới từ kho sản phẩm.</p></div>`;
    $("[data-payment-detail]").innerHTML = `<div class="payment-head"><p class="detail-kicker">ĐƠN #${escapeHtml(order.id.slice(0, 8).toUpperCase())}</p><h2 id="payment-title">${paid ? "Đơn đã hoàn tất" : pending ? "Hoàn tất thanh toán" : "Đơn hàng đã đóng"}</h2><span class="payment-state${statusClass}"><span class="signal-dot"></span>${statusText}</span></div>${paymentContent}`;
    iconRefresh();
    if (pending) startPolling(order.id); else stopPolling();
  }

  function renderDelivery(data) {
    if (!data) return `<div class="delivery-box"><h3>Đã thanh toán, đang chuẩn bị dữ liệu</h3><p class="payment-note">Vui lòng làm mới sau ít giây.</p></div>`;
    const fieldLabels = ["Tài khoản", "Mật khẩu", "2FA", "Email", "Mật khẩu email", "Cookie"];
    const rows = data.split("\n").map(line => line.trim()).filter(Boolean).map(line => {
      const parts = line.split("|");
      if (parts.length >= 2) {
        return `<div class="delivery-item">${parts.map((part, index) => `<div class="delivery-field"><span>${fieldLabels[index] || `Thông tin ${index + 1}`}</span><code>${escapeHtml(part)}</code><button class="copy-button" type="button" data-copy="${escapeHtml(part)}">Sao chép</button></div>`).join("")}</div>`;
      }
      return `<div class="delivery-item"><div class="delivery-field"><span>Thông tin giao hàng</span><code>${escapeHtml(line)}</code><button class="copy-button" type="button" data-copy="${escapeHtml(line)}">Sao chép</button></div></div>`;
    }).join("");
    return `<div class="delivery-box"><h3><i data-lucide="badge-check" aria-hidden="true"></i> Thông tin đã giao</h3>${rows}</div>`;
  }

  function startPolling(orderId) {
    stopPolling();
    state.pollTimer = window.setInterval(async () => {
      try {
        const detail = await api(`/api/shop/orders/${encodeURIComponent(orderId)}`);
        const order = detail.order || detail;
        state.currentOrder = order;
        if (order.status !== "pending") {
          renderPayment(detail);
          loadOrders();
        }
      } catch (_) {
        // The next poll keeps the order view resilient to a transient API failure.
      }
    }, 5000);
  }

  function stopPolling() {
    if (state.pollTimer) window.clearInterval(state.pollTimer);
    state.pollTimer = null;
  }

  async function loadProducts() {
    try {
      state.products = await api("/api/shop/products");
      state.usingFallback = false;
    } catch (_) {
      state.products = fallbackProducts;
      state.usingFallback = true;
    }
    renderCategories();
    filterProducts();
  }

  async function loadCustomer() {
    try {
      state.customer = await api("/api/shop/auth/me");
    } catch (_) {
      state.customer = null;
    }
    updateAuthUi();
    loadOrders();
  }

  function updateAuthUi() {
    $("[data-auth-label]").textContent = state.customer ? state.customer.username : "Đăng nhập";
    $("[data-logout]").hidden = !state.customer;
  }

  async function loadOrders() {
    const panel = $("[data-orders-panel]");
    if (!state.customer) {
      panel.innerHTML = `<div class="empty-state"><i data-lucide="log-in" aria-hidden="true"></i><h3>Đăng nhập để xem đơn</h3><p>Tài khoản của bạn lưu trạng thái thanh toán và thông tin sản phẩm đã mua.</p><button class="primary-button" type="button" data-auth-trigger>Mở đăng nhập <i data-lucide="arrow-up-right" aria-hidden="true"></i></button></div>`;
      iconRefresh();
      return;
    }
    try {
      const orders = await api("/api/shop/orders");
      panel.innerHTML = orders.length
        ? `<div class="order-list">${orders.map(order => `<article class="order-row"><div><h3>${escapeHtml(order.product_name)}</h3><span class="order-code">${escapeHtml(order.id.slice(0, 12).toUpperCase())} · ${new Date(order.created_at).toLocaleDateString("vi-VN")}</span></div><span class="order-amount">${money(order.amount)}</span><span class="status-label status-${escapeHtml(order.status)}">${order.status === "paid" ? "Đã giao" : order.status === "pending" ? "Chờ thanh toán" : escapeHtml(order.status)}</span><button class="order-view-button" type="button" data-order-id="${escapeHtml(order.id)}">Chi tiết</button></article>`).join("")}</div>`
        : `<div class="empty-state"><i data-lucide="package-open" aria-hidden="true"></i><h3>Chưa có đơn hàng</h3><p>Đơn mua tài nguyên MMO của bạn sẽ xuất hiện ở đây.</p><a class="primary-button" href="#catalog">Xem sản phẩm <i data-lucide="arrow-up-right" aria-hidden="true"></i></a></div>`;
    } catch (error) {
      panel.innerHTML = `<div class="empty-state"><i data-lucide="triangle-alert" aria-hidden="true"></i><h3>Không tải được đơn hàng</h3><p>${escapeHtml(error.message)}</p></div>`;
    }
    iconRefresh();
  }

  async function openExistingOrder(id) {
    try {
      const detail = await api(`/api/shop/orders/${encodeURIComponent(id)}`);
      const order = detail.order || detail;
      state.currentOrder = order;
      if (order.status === "pending") showToast("Đơn này vẫn đang chờ thanh toán.");
      renderPayment(detail);
      setModal("payment");
    } catch (error) {
      showToast(error.message, true);
    }
  }

  function setAuthMode(mode) {
    state.authMode = mode;
    $$('[data-auth-mode]').forEach(button => button.classList.toggle("is-active", button.dataset.authMode === mode));
    $("[data-auth-submit]").textContent = mode === "login" ? "Đăng nhập" : "Tạo tài khoản";
    $("[data-auth-form]").querySelector('[name="password"]').autocomplete = mode === "login" ? "current-password" : "new-password";
    $("[data-auth-message]").textContent = "";
  }

  async function submitAuth(event) {
    event.preventDefault();
    const form = event.currentTarget;
    const payload = Object.fromEntries(new FormData(form).entries());
    const message = $("[data-auth-message]");
    message.textContent = "Đang xử lý...";
    message.classList.remove("is-success");
    try {
      state.customer = await api(`/api/shop/auth/${state.authMode}`, { method: "POST", body: JSON.stringify(payload) });
      message.textContent = state.authMode === "login" ? "Đăng nhập thành công." : "Tài khoản đã được tạo.";
      message.classList.add("is-success");
      updateAuthUi();
      loadOrders();
      window.setTimeout(() => {
        if (state.checkoutAfterAuth && state.selectedProduct) {
          state.checkoutAfterAuth = false;
          renderProductDetail();
          setModal("product");
        } else {
          setModal(null);
        }
      }, 450);
    } catch (error) {
      message.textContent = error.message;
    }
  }

  async function logout() {
    await api("/api/shop/auth/logout", { method: "POST" }).catch(() => {});
    state.customer = null;
    updateAuthUi();
    loadOrders();
    setModal(null);
    showToast("Bạn đã đăng xuất.");
  }

  function observeReveals(root = document) {
    const targets = $$('[data-reveal]:not(.is-visible)', root);
    if (!targets.length) return;
    if (!("IntersectionObserver" in window)) {
      targets.forEach(target => target.classList.add("is-visible"));
      return;
    }
    if (!window.__shopRevealObserver) {
      window.__shopRevealObserver = new IntersectionObserver(entries => entries.forEach(entry => {
        if (entry.isIntersecting) {
          entry.target.classList.add("is-visible");
          window.__shopRevealObserver.unobserve(entry.target);
        }
      }), { rootMargin: "0px 0px -8% 0px", threshold: .08 });
    }
    targets.forEach((target, index) => {
      target.style.transitionDelay = `${Math.min(index, 5) * 40}ms`;
      window.__shopRevealObserver.observe(target);
    });
  }

  document.addEventListener("click", event => {
    const productButton = event.target.closest("[data-product-id]");
    if (productButton) openProduct(Number(productButton.dataset.productId));

    const categoryButton = event.target.closest("[data-category]");
    if (categoryButton) {
      state.category = categoryButton.dataset.category;
      renderCategories();
      filterProducts();
    }

    const planButton = event.target.closest("[data-plan-id]");
    if (planButton) {
      state.selectedPlan = state.selectedProduct?.plans?.find(item => item.id === Number(planButton.dataset.planId)) || null;
      renderProductDetail();
    }

    const qtyButton = event.target.closest("[data-qty]");
    if (qtyButton) {
      state.quantity = Math.max(1, Math.min(20, state.quantity + (qtyButton.dataset.qty === "plus" ? 1 : -1)));
      renderProductDetail();
    }

    const orderButton = event.target.closest("[data-order-id]");
    if (orderButton) openExistingOrder(orderButton.dataset.orderId);

    const copyButton = event.target.closest("[data-copy]");
    if (copyButton) {
      navigator.clipboard?.writeText(copyButton.dataset.copy).then(() => showToast("Đã sao chép thông tin.")).catch(() => showToast("Không thể sao chép tự động.", true));
    }

    if (event.target.closest("[data-detail-buy]")) submitOrder();
    if (event.target.closest("[data-auth-trigger]")) setModal("auth");
    if (event.target.closest("[data-how-button]")) setModal("info");
    if (event.target.closest("[data-refresh-orders]")) loadOrders();
    if (event.target.closest("[data-refresh-products]")) loadProducts();
    if (event.target.closest("[data-logout]")) logout();
    if (event.target.closest("[data-close-modal]") || event.target === $("[data-modal-backdrop]")) setModal(null);
  });

  $("[data-auth-form]").addEventListener("submit", submitAuth);
  $$('[data-auth-mode]').forEach(button => button.addEventListener("click", () => setAuthMode(button.dataset.authMode)));
  $("[data-search]").addEventListener("input", filterProducts);
  $$('[data-orders-link]').forEach(link => link.addEventListener("click", event => {
    if (!state.customer) {
      event.preventDefault();
      setModal("auth");
    }
  }));
  document.addEventListener("keydown", event => {
    if (event.key === "Escape") setModal(null);
  });

  updateClock();
  window.setInterval(updateClock, 1000);
  setAuthMode("login");
  observeReveals();
  loadProducts();
  loadCustomer();
  iconRefresh();
})();
