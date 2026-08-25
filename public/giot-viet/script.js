const motionQuery = window.matchMedia("(prefers-reduced-motion: reduce)");
const canvas = document.querySelector("#ocean-canvas");
const ctx = canvas.getContext("2d", { alpha: true });
const hero = document.querySelector(".hero");
const heroProduct = document.querySelector(".hero-product");
const canvasFish = [];
const anchovySprite = new Image();
anchovySprite.src = "assets/anchovy-main.png";

const flavors = {
  anchovy: {
    count: "01 / 03",
    title: "Cá cơm",
    description: "Vị mắm cốt đậm đà, hậu ngọt tự nhiên. Lựa chọn nguyên bản cho căn bếp Việt.",
    protein: "40°N",
    shift: "0%",
  },
  ginger: {
    count: "02 / 03",
    title: "Mắm gừng",
    description: "Vị gừng ấm và dịu, giúp món hấp, món kho thêm thơm và sâu vị.",
    protein: "40°N",
    shift: "-33.333%",
  },
  chili: {
    count: "03 / 03",
    title: "Tỏi ớt",
    description: "Hương tỏi ớt tròn vị, sẵn sàng cho những bữa ăn nhanh mà vẫn đậm đà.",
    protein: "40°N",
    shift: "-66.666%",
  },
};

function resizeCanvas() {
  const ratio = Math.min(window.devicePixelRatio || 1, 2);
  const bounds = hero.getBoundingClientRect();
  canvas.width = Math.floor(bounds.width * ratio);
  canvas.height = Math.floor(bounds.height * ratio);
  canvas.style.width = `${bounds.width}px`;
  canvas.style.height = `${bounds.height}px`;
  ctx.setTransform(ratio, 0, 0, ratio, 0, 0);
}

function createFish(total) {
  canvasFish.length = 0;
  const width = hero.clientWidth;
  const height = hero.clientHeight;

  for (let index = 0; index < total; index += 1) {
    const lane = index % 3;
    canvasFish.push({
      x: width * (0.04 + index * 0.15),
      y: height - 146 + lane * 26,
      size: 30 + lane * 5,
      speed: 0.013 + lane * 0.002,
      drift: index * 0.76,
      opacity: 0.38 + lane * 0.1,
    });
  }
}

function drawFish(fish, time) {
  if (!anchovySprite.complete) return;

  const pulse = Math.sin(time * fish.speed + fish.drift);
  const fishHeight = fish.size * (anchovySprite.height / anchovySprite.width);

  ctx.save();
  ctx.translate(fish.x, fish.y + pulse * 2.5);
  ctx.rotate(pulse * 0.025);
  ctx.scale(-1, 1);
  ctx.globalAlpha = fish.opacity;
  ctx.fillStyle = "#FDBE17";
  ctx.beginPath();
  ctx.moveTo(fish.size * 0.3, 0);
  ctx.lineTo(fish.size * 0.82, -fishHeight * 0.64);
  ctx.lineTo(fish.size * 0.82, fishHeight * 0.64);
  ctx.closePath();
  ctx.fill();
  ctx.drawImage(anchovySprite, -fish.size / 2, -fishHeight / 2, fish.size, fishHeight);
  ctx.restore();
}

function drawWave(time, baseline, amplitude, wavelength, speed, fill, line) {
  const width = hero.clientWidth;
  const height = hero.clientHeight;

  ctx.beginPath();
  ctx.moveTo(0, height);
  for (let x = 0; x <= width + 12; x += 12) {
    const phase = x / wavelength + time * speed;
    const crest = Math.sin(phase) * amplitude + Math.sin(phase * 0.47 + 1.4) * amplitude * 0.45;
    ctx.lineTo(x, baseline + crest);
  }
  ctx.lineTo(width, height);
  ctx.closePath();
  ctx.fillStyle = fill;
  ctx.fill();

  ctx.beginPath();
  for (let x = 0; x <= width + 12; x += 12) {
    const phase = x / wavelength + time * speed;
    const crest = Math.sin(phase) * amplitude + Math.sin(phase * 0.47 + 1.4) * amplitude * 0.45;
    if (x === 0) ctx.moveTo(x, baseline + crest);
    else ctx.lineTo(x, baseline + crest);
  }
  ctx.strokeStyle = line;
  ctx.lineWidth = 1.2;
  ctx.stroke();
}

let animationFrame;
let lastTime = 0;

function animate(time) {
  const width = hero.clientWidth;
  const height = hero.clientHeight;
  ctx.clearRect(0, 0, width, height);

  drawWave(time, height - 178, 15, 128, 0.00044, "#0b4350", "rgba(255, 228, 188, 0.16)");

  canvasFish.forEach((fish) => {
    const delta = lastTime ? Math.min(time - lastTime, 34) : 16;
    fish.x += fish.speed * delta;
    if (fish.x > width + fish.size) fish.x = -fish.size;
    drawFish(fish, time);
  });

  drawWave(time, height - 126, 21, 102, -0.00062, "#0c5862", "rgba(255, 228, 188, 0.19)");
  drawWave(time, height - 72, 13, 76, 0.0008, "#0f6970", "rgba(255, 228, 188, 0.24)");

  lastTime = time;
  animationFrame = requestAnimationFrame(animate);
}

function renderStillFish() {
  const width = hero.clientWidth;
  const height = hero.clientHeight;
  ctx.clearRect(0, 0, width, height);
  drawWave(0, height - 178, 15, 128, 0, "#0b4350", "rgba(255, 228, 188, 0.16)");
  canvasFish.forEach((fish) => drawFish(fish, 0));
  drawWave(0, height - 126, 21, 102, 0, "#0c5862", "rgba(255, 228, 188, 0.19)");
  drawWave(0, height - 72, 13, 76, 0, "#0f6970", "rgba(255, 228, 188, 0.24)");
}

function startOceanMotion() {
  cancelAnimationFrame(animationFrame);
  resizeCanvas();
  createFish(window.innerWidth < 640 ? 5 : 8);
  if (motionQuery.matches) {
    renderStillFish();
    return;
  }
  animationFrame = requestAnimationFrame(animate);
}

window.addEventListener("resize", startOceanMotion, { passive: true });
motionQuery.addEventListener("change", startOceanMotion);
anchovySprite.addEventListener("load", startOceanMotion, { once: true });
startOceanMotion();

if (!motionQuery.matches) {
  let pointerX = 0;
  let pointerY = 0;
  let currentX = 0;
  let currentY = 0;

  hero.addEventListener(
    "pointermove",
    (event) => {
      const bounds = hero.getBoundingClientRect();
      pointerX = (event.clientX - bounds.left) / bounds.width - 0.5;
      pointerY = (event.clientY - bounds.top) / bounds.height - 0.5;
    },
    { passive: true }
  );

  hero.addEventListener("pointerleave", () => {
    pointerX = 0;
    pointerY = 0;
  });

  function floatProduct() {
    currentX += (pointerX - currentX) * 0.045;
    currentY += (pointerY - currentY) * 0.045;
    heroProduct.style.transform = `translate3d(${currentX * 18}px, ${currentY * 14}px, 0) rotateY(${currentX * -2.5}deg)`;
    requestAnimationFrame(floatProduct);
  }

  requestAnimationFrame(floatProduct);
}

const revealObserver = new IntersectionObserver(
  (entries) => {
    entries.forEach((entry) => {
      if (entry.isIntersecting) {
        entry.target.classList.add("is-visible");
        revealObserver.unobserve(entry.target);
      }
    });
  },
  { threshold: 0.15 }
);

document.querySelectorAll(".reveal").forEach((element) => revealObserver.observe(element));

const flavorTabs = document.querySelectorAll(".flavor-tab");
const flavorCount = document.querySelector("#flavor-count");
const flavorName = document.querySelector("#flavor-name");
const flavorDescription = document.querySelector("#flavor-description");
const flavorProtein = document.querySelector("#flavor-protein");
const labelImage = document.querySelector(".label-art img");

flavorTabs.forEach((tab) => {
  tab.addEventListener("click", () => {
    const flavor = flavors[tab.dataset.flavor];
    flavorTabs.forEach((item) => {
      item.classList.toggle("is-active", item === tab);
      item.setAttribute("aria-selected", String(item === tab));
    });

    labelImage.classList.add("is-switching");
    flavorCount.textContent = flavor.count;
    flavorName.textContent = flavor.title;
    flavorDescription.textContent = flavor.description;
    flavorProtein.textContent = flavor.protein;
    labelImage.style.transform = `translateX(${flavor.shift})`;

    window.setTimeout(() => labelImage.classList.remove("is-switching"), 220);
  });
});

const deliveryOrderForm = document.querySelector("#delivery-order-form");
const productOptions = Array.from(document.querySelectorAll(".product-option"));
const orderSummaryItems = document.querySelector("#order-summary-items");
const orderStatus = document.querySelector("#order-status");
const orderSubmit = deliveryOrderForm?.querySelector(".order-submit");
const selectedQuantities = new Map(productOptions.map((option) => [option.dataset.sku, 0]));

function selectedProducts() {
  return productOptions
    .map((option) => {
      const sku = option.dataset.sku;
      const quantity = selectedQuantities.get(sku) || 0;
      return {
        sku,
        name: option.dataset.name,
        quantity,
      };
    })
    .filter((item) => item.quantity > 0);
}

function renderOrderSummary() {
  const products = selectedProducts();
  orderSummaryItems.replaceChildren();

  if (products.length === 0) {
    const empty = document.createElement("li");
    empty.className = "order-summary-empty";
    empty.textContent = "Chưa có sản phẩm nào được chọn.";
    orderSummaryItems.append(empty);
    return;
  }

  products.forEach((product) => {
    const item = document.createElement("li");
    const name = document.createElement("span");
    const quantity = document.createElement("b");
    name.textContent = product.name;
    quantity.textContent = `x${product.quantity}`;
    item.append(name, quantity);
    orderSummaryItems.append(item);
  });
}

function setOrderStatus(message, type = "") {
  orderStatus.textContent = message;
  orderStatus.classList.toggle("is-error", type === "error");
  orderStatus.classList.toggle("is-success", type === "success");
}

function updateQuantity(option, action) {
  const sku = option.dataset.sku;
  const current = selectedQuantities.get(sku) || 0;
  const next = action === "increase" ? Math.min(current + 1, 20) : Math.max(current - 1, 0);
  selectedQuantities.set(sku, next);
  option.querySelector("[data-quantity-for]").value = String(next);
  option.classList.toggle("has-quantity", next > 0);
  renderOrderSummary();
  setOrderStatus("");
}

productOptions.forEach((option) => {
  option.querySelectorAll("[data-quantity-action]").forEach((button) => {
    button.addEventListener("click", () => updateQuantity(option, button.dataset.quantityAction));
  });
});

deliveryOrderForm?.addEventListener("submit", async (event) => {
  event.preventDefault();
  const products = selectedProducts();

  if (products.length === 0) {
    setOrderStatus("Vui lòng chọn ít nhất một sản phẩm trước khi gửi đơn.", "error");
    productOptions[0]?.querySelector(".quantity-button")?.focus();
    return;
  }

  if (!deliveryOrderForm.reportValidity()) {
    return;
  }

  const formData = new FormData(deliveryOrderForm);
  const payload = {
    customer_name: formData.get("customer_name"),
    phone: formData.get("phone"),
    province: formData.get("province"),
    ward: formData.get("ward"),
    address: formData.get("address"),
    delivery_note: formData.get("delivery_note"),
    website: formData.get("website"),
    items: products.map(({ sku, quantity }) => ({ sku, quantity })),
  };

  orderSubmit.disabled = true;
  orderSubmit.textContent = "Đang gửi đơn...";
  setOrderStatus("Đơn hàng đang được gửi đến Giọt Việt.");

  try {
    const response = await fetch("/api/giot-viet/orders", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(payload),
    });
    const result = await response.json().catch(() => null);

    if (!response.ok || !result?.ok) {
      throw new Error(result?.error?.message || "Không thể gửi đơn hàng.");
    }

    deliveryOrderForm.reset();
    selectedQuantities.forEach((_, sku) => selectedQuantities.set(sku, 0));
    productOptions.forEach((option) => {
      option.classList.remove("has-quantity");
      option.querySelector("[data-quantity-for]").value = "0";
    });
    renderOrderSummary();
    setOrderStatus(`Đặt hàng thành công. Mã đơn của bạn là ${result.data.order_id}. Giọt Việt sẽ liên hệ để xác nhận.`, "success");
  } catch (error) {
    setOrderStatus("Chưa thể gửi đơn. Vui lòng kiểm tra kết nối và thử lại.", "error");
  } finally {
    orderSubmit.disabled = false;
    orderSubmit.textContent = "Xác nhận đặt hàng";
  }
});

renderOrderSummary();
