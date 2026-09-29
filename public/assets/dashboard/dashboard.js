const STORAGE_KEY = "chocode_api_key";

const demoProducts = [
  {
    id: "demo-1",
    name: "Bộ công cụ tự động hóa",
    price: 79000,
    category: "Tool / Script",
    description: "Các script nền tảng cho những tác vụ lặp lại cần xử lý nhanh và gọn.",
    delivery_type: "text",
    stock_count: 18,
    image: "/images/buoc1.png",
  },
  {
    id: "demo-2",
    name: "Landing kit cho dự án nhỏ",
    price: 149000,
    category: "Website / Web App",
    description: "Bộ tài nguyên giúp bạn bắt đầu một trang giới thiệu rõ ràng, dễ chỉnh sửa.",
    delivery_type: "file",
    stock_count: 7,
    image: "/images/buoc2.png",
  },
  {
    id: "demo-3",
    name: "MMO workflow starter",
    price: 99000,
    category: "MMO Tool",
    description: "Quy trình khởi đầu để sắp xếp công việc, tài khoản và những bước lặp lại.",
    delivery_type: "text",
    stock_count: 24,
    image: "/images/buoc3.png",
  },
  {
    id: "demo-4",
    name: "Telegram helper pack",
    price: 119000,
    category: "Bot / Automation",
    description: "Mẫu lệnh và cấu trúc tiện ích cho những bot Telegram cần vận hành ổn định.",
    delivery_type: "manual_input",
    stock_count: 0,
    plans: [
      { id: "demo-plan-1", label: "1 tháng", months: 1, price: 119000 },
      { id: "demo-plan-2", label: "3 tháng", months: 3, price: 279000 },
    ],
    input_prompt: "Nhập thông tin cần dùng để kích hoạt",
    image: "/images/buoc4.png",
  },
  {
    id: "demo-5",
    name: "Data utility mini pack",
    price: 59000,
    category: "Tool / Script",
    description: "Nhóm tiện ích nhỏ cho việc chuẩn hóa và kiểm tra dữ liệu hàng ngày.",
    delivery_type: "text",
    stock_count: 32,
    image: "/images/buoc5.png",
  },
  {
    id: "demo-6",
    name: "Starter pack cho creator",
    price: 89000,
    category: "Khác",
    description: "Tài nguyên gọn để sắp xếp ý tưởng, nội dung và tiến độ làm việc cá nhân.",
    delivery_type: "file",
    stock_count: 12,
    image: "/images/buoc6.png",
  },
];

const state = {
  apiKey: localStorage.getItem(STORAGE_KEY) || "",
  products: [],
  category: "Tất cả",
  query: "",
  wallet: null,
  selectedProduct: null,
};

const $ = (selector, root = document) => root.querySelector(selector);
const $$ = (selector, root = document) => Array.from(root.querySelectorAll(selector));

function escapeHtml(value) {
  return String(value ?? "")
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#039;");
}

function formatVnd(value) {
  const amount = Number(value || 0);
  return `${new Intl.NumberFormat("vi-VN").format(amount)}đ`;
}

function imageUrl(product) {
  const value = product.image_url || product.image || "";
  if (!value) return "";
  if (value.startsWith("/") || value.startsWith("https://") || value.startsWith("http://")) return value;
  return `/${value}`;
}

function deliveryLabel(product) {
  if (product.delivery_type === "manual_input" || Number(product.requires_input) === 1) return "Kích hoạt theo thông tin";
  if (product.delivery_type === "file") return "Giao file sau mua";
  return "Giao tự động";
}

function productCode(product) {
  const source = String(product.id || "00").replace(/[^a-z0-9]/gi, "").slice(-2).toUpperCase();
  return source.padStart(2, "0");
}

function placeholderMarkup(product) {
  return `<div class="product-placeholder" aria-hidden="true"><span class="placeholder-label">CODE / ${escapeHtml(productCode(product))}</span><span class="placeholder-block"></span></div>`;
}

function productMediaMarkup(product) {
  const image = imageUrl(product);
  return `<div class="product-media">${image ? `<img src="${escapeHtml(image)}" alt="${escapeHtml(product.name)}" loading="lazy">` : placeholderMarkup(product)}<span class="product-badge">${escapeHtml(deliveryLabel(product))}</span></div>`;
}

function getFilteredProducts() {
  const query = state.query.trim().toLowerCase();
  return state.products.filter((product) => {
    const matchesCategory = state.category === "Tất cả" || (product.category || "Khác") === state.category;
    const searchable = `${product.name} ${product.description || ""} ${product.category || ""}`.toLowerCase();
    return matchesCategory && (!query || searchable.includes(query));
  });
}

function renderCategories() {
  const categories = ["Tất cả", ...new Set(state.products.map((product) => product.category || "Khác"))];
  $("#category-list").innerHTML = categories.map((category) => {
    const count = category === "Tất cả"
      ? state.products.length
      : state.products.filter((product) => (product.category || "Khác") === category).length;
    return `<button class="category-button${state.category === category ? " is-active" : ""}" data-category="${escapeHtml(category)}" type="button">${escapeHtml(category)} <span>${count}</span></button>`;
  }).join("");
}

function renderProducts() {
  const grid = $("#products-grid");
  const products = getFilteredProducts();
  const total = state.products.length;
  const mode = state.apiKey ? "Đã kết nối với cửa hàng" : "Đang xem sản phẩm mẫu";
  $("#catalog-mode").textContent = mode;

  if (!products.length) {
    grid.innerHTML = `<div class="empty-state"><strong>Không tìm thấy sản phẩm phù hợp.</strong><p>Thử từ khóa khác hoặc đặt lại bộ lọc.</p></div>`;
  } else {
    grid.innerHTML = products.map((product) => {
      const stock = Number(product.stock_count || 0);
      const stockLabel = product.delivery_type === "manual_input" ? "Theo gói" : `${stock} sản phẩm còn lại`;
      return `<article class="product-card reveal-on-scroll" data-product-id="${escapeHtml(product.id)}">
        ${productMediaMarkup(product)}
        <div class="product-body">
          <div class="product-meta"><span>${escapeHtml(product.category || "Tài nguyên số")}</span><span>${escapeHtml(stockLabel)}</span></div>
          <h3>${escapeHtml(product.name)}</h3>
          <p class="product-description">${escapeHtml(product.description || "Tài nguyên số sẵn sàng để sử dụng.")}</p>
          <div class="product-footer"><div class="product-price"><small>Giá từ</small><strong>${formatVnd(product.price)}</strong></div><button class="product-cta" type="button" data-open-product="${escapeHtml(product.id)}">Xem sản phẩm ↗</button></div>
        </div>
      </article>`;
    }).join("");
  }

  const visibleText = products.length === total ? `${total} sản phẩm trong cửa hàng` : `${products.length} trên ${total} sản phẩm`;
  $("#catalog-status").textContent = visibleText;
  observeReveals();
}

function setDemoProducts() {
  state.products = demoProducts;
  state.category = "Tất cả";
  renderCategories();
  renderProducts();
}

async function apiFetch(path, options = {}) {
  if (!state.apiKey) throw new Error("Chưa có API key");
  const response = await fetch(path, {
    ...options,
    headers: {
      ...(options.headers || {}),
      Authorization: `Bearer ${state.apiKey}`,
      "Content-Type": "application/json",
    },
  });
  const payload = await response.json().catch(() => ({}));
  if (!response.ok || payload.ok === false) {
    throw new Error(payload.error?.message || "Không thể kết nối đến cửa hàng.");
  }
  return payload.data;
}

async function loadConnectedStore() {
  if (!state.apiKey) return;
  try {
    const [products, wallet] = await Promise.all([
      apiFetch("/api/client/products"),
      apiFetch("/api/client/wallet"),
    ]);
    state.products = Array.isArray(products) ? products : [];
    state.wallet = wallet;
    updateWallet(wallet);
    renderCategories();
    renderProducts();
    showToast("Đã kết nối cửa hàng");
  } catch (error) {
    updateWallet(null);
    showToast(error.message || "Không thể tải dữ liệu cửa hàng.");
    setDemoProducts();
  }
}

function updateWallet(wallet) {
  const value = wallet?.balance_display || (wallet ? formatVnd(wallet.balance) : "Chưa kết nối");
  $("#wallet-balance").textContent = value;
}

function openModal(id) {
  const modal = $(`#${id}`);
  if (!modal) return;
  modal.hidden = false;
  modal.classList.add("is-open");
  modal.setAttribute("aria-hidden", "false");
  document.body.style.overflow = "hidden";
  const focusTarget = modal.querySelector("input, textarea, select, button");
  window.setTimeout(() => focusTarget?.focus(), 50);
}

function closeModal(modal) {
  const target = typeof modal === "string" ? $(`#${modal}`) : modal;
  if (!target) return;
  target.classList.remove("is-open");
  target.setAttribute("aria-hidden", "true");
  target.hidden = true;
  if (!$$(".modal.is-open").length) document.body.style.overflow = "";
}

function openConnectModal(message = "") {
  $("#connect-error").textContent = message;
  $("#api-key-input").value = state.apiKey;
  openModal("connect-modal");
}

function renderProductModal(product) {
  const manual = product.delivery_type === "manual_input" || Number(product.requires_input) === 1;
  const plans = Array.isArray(product.plans) ? product.plans : [];
  const image = imageUrl(product);
  const imageMarkup = image ? `<img src="${escapeHtml(image)}" alt="${escapeHtml(product.name)}">` : placeholderMarkup(product);
  const planMarkup = manual && plans.length ? `<div class="purchase-field"><label>Chọn gói</label><div class="plan-picker">${plans.map((plan, index) => `<div class="plan-option"><input id="plan-${escapeHtml(plan.id)}" type="radio" name="plan_id" value="${escapeHtml(plan.id)}" ${index === 0 ? "checked" : ""}><label for="plan-${escapeHtml(plan.id)}"><strong>${escapeHtml(plan.label)}</strong><small>${formatVnd(plan.price)}</small></label></div>`).join("")}</div></div>` : "";
  const inputMarkup = manual ? `<div class="purchase-field"><label for="customer-input">${escapeHtml(product.input_prompt || "Thông tin kích hoạt")}</label><textarea id="customer-input" name="customer_input" placeholder="Nhập thông tin cần dùng để kích hoạt"></textarea></div>` : "";
  const quantityMarkup = !manual ? `<div class="purchase-field"><label for="purchase-qty">Số lượng</label><input id="purchase-qty" name="qty" type="number" min="1" max="${Math.max(1, Number(product.stock_count || 1))}" value="1"></div>` : "";

  $("#product-modal-content").innerHTML = `<div class="product-modal-inner">
    <button class="modal-close" type="button" data-close-modal aria-label="Đóng">×</button>
    <div class="product-modal-head">
      <div class="product-modal-media">${imageMarkup}</div>
      <div class="product-modal-copy"><p class="eyebrow">${escapeHtml(product.category || "Tài nguyên số")}</p><h2 id="product-modal-title">${escapeHtml(product.name)}</h2><p>${escapeHtml(product.description || "Tài nguyên số sẵn sàng để sử dụng.")}</p><div class="product-info-line"><span class="info-tag">${escapeHtml(deliveryLabel(product))}</span><span class="info-tag">${manual ? "Chọn gói phù hợp" : `${Number(product.stock_count || 0)} còn lại`}</span></div></div>
    </div>
    <form class="purchase-form" id="purchase-form">
      ${manual ? planMarkup : `<div class="purchase-fields"><div class="purchase-field"><label>Đơn giá</label><input type="text" value="${formatVnd(product.price)}" readonly></div>${quantityMarkup}</div>`}
      ${inputMarkup}
      <div class="form-error" id="purchase-error" role="alert"></div>
      <button class="primary-button purchase-submit" type="submit">${state.apiKey ? "Mua bằng số dư ví" : "Kết nối để mua"}<span class="button-arrow" aria-hidden="true">↗</span></button>
      <div id="purchase-result"></div>
    </form>
  </div>`;
  openModal("product-modal");
  $("#purchase-form").addEventListener("submit", (event) => submitPurchase(event, product));
}

async function submitPurchase(event, product) {
  event.preventDefault();
  if (!state.apiKey) {
    closeModal("product-modal");
    openConnectModal();
    return;
  }

  const form = event.currentTarget;
  const button = $(".purchase-submit", form);
  const error = $("#purchase-error", form);
  const result = $("#purchase-result", form);
  const manual = product.delivery_type === "manual_input" || Number(product.requires_input) === 1;
  const selectedPlan = form.querySelector('input[name="plan_id"]:checked')?.value;
  const quantity = Number($("#purchase-qty", form)?.value || 1);
  const customerInput = $("#customer-input", form)?.value.trim() || undefined;

  if (manual && !customerInput) {
    error.textContent = "Vui lòng nhập thông tin kích hoạt.";
    $("#customer-input", form)?.focus();
    return;
  }

  error.textContent = "";
  result.innerHTML = "";
  button.disabled = true;
  button.firstChild.textContent = "Đang xử lý...";

  try {
    const order = await apiFetch("/api/client/orders", {
      method: "POST",
      body: JSON.stringify({
        product_id: product.id,
        qty: manual ? 1 : quantity,
        plan_id: selectedPlan ? Number(selectedPlan) : undefined,
        customer_input: customerInput,
      }),
    });
    state.wallet = { balance: order.balance_after, balance_display: formatVnd(order.balance_after) };
    updateWallet(state.wallet);
    result.innerHTML = `<div class="purchase-result"><strong>Đơn hàng đã hoàn tất.</strong><span>Mã đơn: ${escapeHtml(order.order_id || "-")}</span>${order.delivered_data ? `<textarea class="delivery-data" readonly aria-label="Dữ liệu đã giao">${escapeHtml(order.delivered_data)}</textarea>` : ""}</div>`;
    button.style.display = "none";
    showToast("Mua hàng thành công");
  } catch (purchaseError) {
    error.textContent = purchaseError.message || "Không thể hoàn tất đơn hàng.";
    button.disabled = false;
    button.firstChild.textContent = "Mua bằng số dư ví";
  }
}

function showToast(message) {
  const toast = $("#toast");
  toast.textContent = message;
  toast.classList.add("is-visible");
  window.clearTimeout(showToast.timer);
  showToast.timer = window.setTimeout(() => toast.classList.remove("is-visible"), 3400);
}

function observeReveals() {
  const elements = $$(".reveal-on-scroll:not(.is-observed)");
  if (!elements.length) return;
  if (!("IntersectionObserver" in window)) {
    elements.forEach((element) => element.classList.add("is-visible", "is-observed"));
    return;
  }
  const observer = new IntersectionObserver((entries, currentObserver) => {
    entries.forEach((entry) => {
      if (!entry.isIntersecting) return;
      entry.target.classList.add("is-visible", "is-observed");
      currentObserver.unobserve(entry.target);
    });
  }, { threshold: .14 });
  elements.forEach((element) => observer.observe(element));
}

function bindNavigation() {
  $("#menu-toggle").addEventListener("click", () => {
    const button = $("#menu-toggle");
    const menu = $("#mobile-nav");
    const isOpen = button.classList.toggle("is-open");
    menu.classList.toggle("is-open", isOpen);
    button.setAttribute("aria-expanded", String(isOpen));
    menu.setAttribute("aria-hidden", String(!isOpen));
  });

  $$("#mobile-nav a").forEach((link) => link.addEventListener("click", () => {
    $("#menu-toggle").classList.remove("is-open");
    $("#mobile-nav").classList.remove("is-open");
    $("#menu-toggle").setAttribute("aria-expanded", "false");
    $("#mobile-nav").setAttribute("aria-hidden", "true");
  }));

  $("#browse-products").addEventListener("click", () => $("#products").scrollIntoView({ behavior: "smooth", block: "start" }));
  $("#hero-search-form").addEventListener("submit", (event) => {
    event.preventDefault();
    state.query = $("#hero-search").value;
    renderProducts();
    $("#products").scrollIntoView({ behavior: "smooth", block: "start" });
  });
  $("#hero-search").addEventListener("input", (event) => {
    state.query = event.currentTarget.value;
    renderProducts();
  });
  $("#clear-filters").addEventListener("click", () => {
    state.query = "";
    state.category = "Tất cả";
    $("#hero-search").value = "";
    renderCategories();
    renderProducts();
  });
  $("#category-list").addEventListener("click", (event) => {
    const button = event.target.closest("[data-category]");
    if (!button) return;
    state.category = button.dataset.category;
    renderCategories();
    renderProducts();
  });
  $("#products-grid").addEventListener("click", (event) => {
    const trigger = event.target.closest("[data-open-product]");
    const card = event.target.closest("[data-product-id]");
    const productId = trigger?.dataset.openProduct || card?.dataset.productId;
    if (!productId) return;
    const product = state.products.find((item) => String(item.id) === String(productId));
    if (product) renderProductModal(product);
  });
}

function bindModals() {
  $("#open-connect").addEventListener("click", () => openConnectModal());
  $("#connect-cta").addEventListener("click", () => openConnectModal());
  document.addEventListener("click", (event) => {
    if (event.target.closest("[data-close-modal]")) {
      closeModal(event.target.closest(".modal"));
    }
    const closeButton = event.target.closest(".modal-close");
    if (closeButton) closeModal(closeButton.closest(".modal"));
  });
  document.addEventListener("keydown", (event) => {
    if (event.key === "Escape") $$(".modal.is-open").forEach(closeModal);
  });

  $("#connect-form").addEventListener("submit", async (event) => {
    event.preventDefault();
    const input = $("#api-key-input");
    const error = $("#connect-error");
    const button = event.currentTarget.querySelector("button[type=submit]");
    const key = input.value.trim();
    if (!key) {
      error.textContent = "Vui lòng nhập API key.";
      return;
    }
    state.apiKey = key;
    localStorage.setItem(STORAGE_KEY, key);
    button.disabled = true;
    button.firstChild.textContent = "Đang kiểm tra...";
    error.textContent = "";
    try {
      const [products, wallet] = await Promise.all([apiFetch("/api/client/products"), apiFetch("/api/client/wallet")]);
      state.products = Array.isArray(products) ? products : [];
      state.wallet = wallet;
      updateWallet(wallet);
      renderCategories();
      renderProducts();
      closeModal("connect-modal");
      showToast("Kết nối thành công");
    } catch (connectError) {
      error.textContent = connectError.message || "API key không hợp lệ hoặc server chưa sẵn sàng.";
      state.apiKey = "";
      localStorage.removeItem(STORAGE_KEY);
      setDemoProducts();
    } finally {
      button.disabled = false;
      button.firstChild.textContent = "Lưu và kiểm tra";
    }
  });

  $("#disconnect-key").addEventListener("click", () => {
    state.apiKey = "";
    state.wallet = null;
    localStorage.removeItem(STORAGE_KEY);
    updateWallet(null);
    setDemoProducts();
    closeModal("connect-modal");
    showToast("Đã xóa API key khỏi trình duyệt");
  });
}

function showInitialMotion() {
  requestAnimationFrame(() => {
    $$(".reveal-on-load").forEach((element) => element.classList.add("is-visible"));
    observeReveals();
  });
}

bindNavigation();
bindModals();
setDemoProducts();
showInitialMotion();
if (state.apiKey) loadConnectedStore();
