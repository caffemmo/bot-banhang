import * as THREE from "./assets/vendor/three.module.js";

const viewer = document.querySelector("#bottle-viewer");
const product = document.querySelector(".hero-product");
const reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)");

if (viewer && product) {
  initBottleViewer().catch(() => {
    product.classList.remove("has-3d-viewer");
  });
}

async function initBottleViewer() {
  const labelSource = await loadImage("assets/product-labels.png");
  const renderer = new THREE.WebGLRenderer({ alpha: true, antialias: true, powerPreference: "high-performance" });
  const scene = new THREE.Scene();
  const camera = new THREE.PerspectiveCamera(33, 1, 0.1, 100);
  const bottle = createBottle(labelSource, renderer);
  const shadow = createShadow();
  let animationFrame = 0;
  let isVisible = true;
  let isDragging = false;
  let pointerId = null;
  let lastPointer = { x: 0, y: 0 };
  let yaw = 0;
  let pitch = 0.03;
  let targetYaw = 0;
  let targetPitch = pitch;
  let lastInteraction = performance.now();
  let lastFrame = performance.now();

  renderer.outputColorSpace = THREE.SRGBColorSpace;
  renderer.setClearColor(0x000000, 0);
  renderer.domElement.setAttribute("aria-hidden", "true");
  viewer.append(renderer.domElement);

  scene.add(new THREE.HemisphereLight(0xffe4bc, 0x082a32, 2.5));

  const keyLight = new THREE.DirectionalLight(0xffe2ac, 3.2);
  keyLight.position.set(4.5, 7, 8);
  scene.add(keyLight);

  const rimLight = new THREE.DirectionalLight(0xffbe4a, 2.6);
  rimLight.position.set(-5, 2, -5);
  scene.add(rimLight);

  const fillLight = new THREE.PointLight(0x78d0da, 14, 15, 2);
  fillLight.position.set(-3.5, 0.5, 5);
  scene.add(fillLight, shadow, bottle);

  const resizeObserver = new ResizeObserver(resize);
  resizeObserver.observe(viewer);

  const visibilityObserver = new IntersectionObserver(
    ([entry]) => {
      isVisible = entry.isIntersecting;
      if (isVisible) queueRender();
    },
    { threshold: 0.05 }
  );
  visibilityObserver.observe(viewer);

  viewer.addEventListener("pointerdown", onPointerDown);
  viewer.addEventListener("pointermove", onPointerMove);
  viewer.addEventListener("pointerup", onPointerUp);
  viewer.addEventListener("pointercancel", onPointerUp);
  viewer.addEventListener("keydown", onKeyDown);
  viewer.addEventListener("webglcontextlost", onContextLost, false);

  product.classList.add("has-3d-viewer");
  resize();
  queueRender();

  function resize() {
    const width = Math.max(viewer.clientWidth, 1);
    const height = Math.max(viewer.clientHeight, 1);
    const verticalFov = THREE.MathUtils.degToRad(camera.fov);
    const viewDistance = 9.6 / (2 * Math.tan(verticalFov / 2));

    camera.aspect = width / height;
    camera.position.set(0, 0.25, viewDistance * 1.08);
    camera.lookAt(0, 0.1, 0);
    camera.updateProjectionMatrix();
    renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 1.75));
    renderer.setSize(width, height, false);
    queueRender();
  }

  function onPointerDown(event) {
    isDragging = true;
    pointerId = event.pointerId;
    lastPointer = { x: event.clientX, y: event.clientY };
    lastInteraction = performance.now();
    viewer.setPointerCapture(pointerId);
    queueRender();
  }

  function onPointerMove(event) {
    if (!isDragging || event.pointerId !== pointerId) return;

    targetYaw += (event.clientX - lastPointer.x) * 0.012;
    targetPitch = THREE.MathUtils.clamp(targetPitch + (event.clientY - lastPointer.y) * 0.004, -0.18, 0.2);
    lastPointer = { x: event.clientX, y: event.clientY };
    lastInteraction = performance.now();
    queueRender();
  }

  function onPointerUp(event) {
    if (event.pointerId !== pointerId) return;
    isDragging = false;
    if (viewer.hasPointerCapture(pointerId)) viewer.releasePointerCapture(pointerId);
    pointerId = null;
    lastInteraction = performance.now();
    queueRender();
  }

  function onKeyDown(event) {
    const keyStep = 0.23;
    if (event.key === "ArrowLeft") targetYaw -= keyStep;
    else if (event.key === "ArrowRight") targetYaw += keyStep;
    else if (event.key === "ArrowUp") targetPitch = THREE.MathUtils.clamp(targetPitch - 0.08, -0.18, 0.2);
    else if (event.key === "ArrowDown") targetPitch = THREE.MathUtils.clamp(targetPitch + 0.08, -0.18, 0.2);
    else return;

    event.preventDefault();
    lastInteraction = performance.now();
    queueRender();
  }

  function queueRender() {
    if (!animationFrame && isVisible) animationFrame = requestAnimationFrame(renderFrame);
  }

  function renderFrame(time) {
    animationFrame = 0;
    if (!isVisible) return;

    const elapsed = Math.min((time - lastFrame) / 1000, 0.05);
    lastFrame = time;
    if (!reduceMotion.matches && !isDragging && time - lastInteraction > 900) targetYaw += elapsed * 0.055;

    yaw += (targetYaw - yaw) * 0.14;
    pitch += (targetPitch - pitch) * 0.14;
    bottle.rotation.y = yaw;
    bottle.rotation.x = pitch;
    bottle.userData.label.visible = Math.cos(yaw) > -0.04;
    shadow.rotation.z = yaw * -0.08;
    renderer.render(scene, camera);

    if (!reduceMotion.matches || isDragging || Math.abs(targetYaw - yaw) > 0.001 || Math.abs(targetPitch - pitch) > 0.001) {
      queueRender();
    }
  }

  function onContextLost(event) {
    event.preventDefault();
    product.classList.remove("has-3d-viewer");
    resizeObserver.disconnect();
    visibilityObserver.disconnect();
    cancelAnimationFrame(animationFrame);
  }
}

function createBottle(labelSource, renderer) {
  const bottle = new THREE.Group();
  bottle.position.y = -0.18;

  const glass = new THREE.MeshPhysicalMaterial({
    color: 0xddeff0,
    roughness: 0.08,
    metalness: 0,
    transmission: 0.35,
    transparent: true,
    opacity: 0.3,
    ior: 1.42,
    side: THREE.DoubleSide,
    depthWrite: false,
  });
  const fishSauce = new THREE.MeshPhysicalMaterial({
    color: 0x7d1a09,
    roughness: 0.22,
    metalness: 0,
    clearcoat: 0.55,
    clearcoatRoughness: 0.18,
  });
  const collar = new THREE.MeshPhysicalMaterial({
    color: 0xe4f3f2,
    roughness: 0.16,
    transmission: 0.18,
    transparent: true,
    opacity: 0.78,
  });
  const capMaterial = new THREE.MeshStandardMaterial({ color: 0xfdad16, roughness: 0.35, metalness: 0.12 });
  const capHighlight = new THREE.MeshStandardMaterial({ color: 0xffcd45, roughness: 0.29, metalness: 0.08 });
  const bandMaterial = new THREE.MeshStandardMaterial({ color: 0x8c4d10, roughness: 0.35, metalness: 0.18 });

  const bodyPoints = [
    new THREE.Vector2(1.5, -3.95),
    new THREE.Vector2(1.67, -3.82),
    new THREE.Vector2(1.73, -3.48),
    new THREE.Vector2(1.73, -1.85),
    new THREE.Vector2(1.66, -1.02),
    new THREE.Vector2(1.51, -0.38),
    new THREE.Vector2(1.12, 0.34),
    new THREE.Vector2(0.9, 0.84),
    new THREE.Vector2(0.8, 2.95),
    new THREE.Vector2(0.77, 3.3),
  ];
  const saucePoints = bodyPoints.map((point) => new THREE.Vector2(point.x * 0.91, point.y + 0.08));
  saucePoints.splice(-2, 2, new THREE.Vector2(0.72, 2.45), new THREE.Vector2(0.7, 2.62));

  const sauce = new THREE.Mesh(new THREE.LatheGeometry(saucePoints, 72), fishSauce);
  const body = new THREE.Mesh(new THREE.LatheGeometry(bodyPoints, 72), glass);
  const neck = new THREE.Mesh(new THREE.CylinderGeometry(0.76, 0.8, 0.76, 72), glass);
  neck.position.y = 3.63;
  const sauceTop = new THREE.Mesh(new THREE.CylinderGeometry(0.69, 0.69, 0.025, 72), fishSauce);
  sauceTop.position.y = 2.64;
  const neckCollar = new THREE.Mesh(new THREE.CylinderGeometry(0.81, 0.81, 0.44, 72), collar);
  neckCollar.position.y = 4.0;

  const label = createLabel(labelSource, renderer);
  bottle.userData.label = label;
  bottle.add(sauce, sauceTop, body, neck, neckCollar, label, createCap(capMaterial, capHighlight, bandMaterial));
  return bottle;
}

function createLabel(labelSource, renderer) {
  const labelCanvas = document.createElement("canvas");
  labelCanvas.width = 768;
  labelCanvas.height = 1600;
  const context = labelCanvas.getContext("2d");

  // The source contains all label variants. This crop is the Ca Com front label.
  context.drawImage(labelSource, 553, 26, 244, 638, 0, 0, labelCanvas.width, labelCanvas.height);
  maskLabelCanvas(context, labelCanvas.width, labelCanvas.height);

  const texture = new THREE.CanvasTexture(labelCanvas);
  texture.colorSpace = THREE.SRGBColorSpace;
  texture.anisotropy = Math.min(renderer.capabilities.getMaxAnisotropy(), 8);
  const material = new THREE.SpriteMaterial({
    map: texture,
    transparent: true,
    depthTest: false,
    depthWrite: false,
  });
  const label = new THREE.Sprite(material);
  label.scale.set(2.26, 4.18, 1);
  label.position.set(0, -1.12, 1.82);
  label.renderOrder = 8;
  return label;
}

function maskLabelCanvas(context, width, height) {
  context.save();
  context.globalCompositeOperation = "destination-in";
  context.beginPath();
  context.moveTo(width * 0.1, height);
  context.quadraticCurveTo(0, height * 0.96, 0, height * 0.89);
  context.lineTo(0, height * 0.14);
  context.quadraticCurveTo(width * 0.02, height * 0.02, width * 0.19, 0);
  context.lineTo(width * 0.81, 0);
  context.quadraticCurveTo(width * 0.98, height * 0.02, width, height * 0.14);
  context.lineTo(width, height * 0.89);
  context.quadraticCurveTo(width, height * 0.96, width * 0.9, height);
  context.closePath();
  context.fill();
  context.restore();
}

function createCap(capMaterial, capHighlight, bandMaterial) {
  const cap = new THREE.Group();
  const main = new THREE.Mesh(new THREE.CylinderGeometry(0.84, 0.9, 0.56, 72), capMaterial);
  main.position.y = 4.53;
  const top = new THREE.Mesh(new THREE.CylinderGeometry(0.77, 0.84, 0.1, 72), capHighlight);
  top.position.y = 4.86;
  const band = new THREE.Mesh(new THREE.CylinderGeometry(0.84, 0.84, 0.09, 72), bandMaterial);
  band.position.y = 4.19;
  cap.add(main, top, band);

  for (let index = 0; index < 20; index += 1) {
    const angle = (index / 20) * Math.PI * 2;
    const ridge = new THREE.Mesh(new THREE.BoxGeometry(0.07, 0.38, 0.065), capHighlight);
    ridge.position.set(Math.sin(angle) * 0.88, 4.53, Math.cos(angle) * 0.88);
    ridge.rotation.y = -angle;
    cap.add(ridge);
  }

  return cap;
}

function createShadow() {
  const material = new THREE.MeshBasicMaterial({ color: 0x120605, transparent: true, opacity: 0.33, depthWrite: false });
  const shadow = new THREE.Mesh(new THREE.CircleGeometry(2.38, 72), material);
  shadow.scale.set(1, 0.33, 1);
  shadow.rotation.x = -Math.PI / 2;
  shadow.position.y = -4.08;
  return shadow;
}

function loadImage(source) {
  return new Promise((resolve, reject) => {
    const image = new Image();
    image.onload = () => resolve(image);
    image.onerror = reject;
    image.src = source;
  });
}
