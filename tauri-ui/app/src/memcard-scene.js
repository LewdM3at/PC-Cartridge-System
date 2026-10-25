/**
 * The memory card's 3D stage: a card in the middle, and the saves on an orbit
 * round it.
 *
 * This file is presentation only. It is given elements and an index, and puts
 * them in space; it knows nothing about saves, copies or the backend, so the
 * memory card module decides what is chosen and this decides how it looks.
 *
 * Built from CSS 3D transforms rather than WebGL. The launcher on Linux runs in
 * WebKitGTK, where WebGL is often software-rendered or switched off, and that
 * is exactly the Steam Deck and living-room machine this is for. Transformed
 * DOM is composited on the GPU in every webview Tauri uses, takes the real
 * cover art as it is, and needs no 3D library in a frontend that has no
 * bundler.
 *
 * Layout. The saves stand on a circular track seen from slightly above, each
 * facing out from the centre, so the chosen one faces you at the front and its
 * neighbours turn away round the curve. The physical card hovers over the
 * centre. From five to thirteen saves close a ring round the card; fewer make a
 * short arc (a ring of three would be mostly gaps), and more make an arc whose
 * far ends fade. An arc stops at its ends rather than wrapping: spinning thirty
 * cases past to get from the last to the first is not a way to find anything.
 *
 * Motion. Everything is a spring towards a target, and the targets come from
 * the selection alone, so however fast a pad repeats, what you see always ends
 * on what is chosen. A frame is drawn only while something is still moving:
 * once the ring settles and the pointer is still, nothing runs.
 */

/** How far apart neighbours sit on an arc, in degrees. */
const STEP_ARC = 40;
const STEP_MIN = 26;
/** Fewer saves than this make an arc; a ring of them would be mostly gaps. */
const RING_FROM = 5;
/** Saves fade out from FADE_FROM degrees off the front and are gone by VISIBLE. */
const FADE_FROM = 90;
const VISIBLE = 124;

const still = matchMedia("(prefers-reduced-motion: reduce)");

/**
 * Whether 3D will really be drawn. Supporting the property is not enough:
 * WebKitGTK without a GPU (a VM, some NVIDIA setups) paints 3D transforms
 * flattened, every face squashed onto the page with no depth and no back
 * faces hidden. WebKit's own `-webkit-transform-3d` media feature is true only
 * when its compositor can render 3D, so it says when to lay the track flat
 * instead. A webview that does not know the feature (it parses as `not all`)
 * is taken at its word.
 */
function canDraw3d() {
  if (!CSS.supports("transform-style", "preserve-3d")) return false;
  const query = matchMedia("(-webkit-transform-3d)");
  return query.media === "not all" || query.matches;
}

const flat = !canDraw3d();

/** A spring: `x` chases `to`, a little under-damped so it lands, not stops. */
function spring(x = 0) {
  return { x, v: 0, to: x };
}

function stepSpring(s, dt, stiffness = 165, damping = 21) {
  const force = stiffness * (s.to - s.x) - damping * s.v;
  s.v += force * dt;
  s.x += s.v * dt;
}

function settled(s) {
  return Math.abs(s.to - s.x) < 0.0008 && Math.abs(s.v) < 0.0008;
}

function snap(s) {
  s.x = s.to;
  s.v = 0;
}

/**
 * The card itself: a slab of moulded plastic with a grip at the top, a recessed
 * label, a moulded arrow and the connector along its bottom edge.
 */
export function buildCard() {
  const card = document.createElement("div");
  card.className = "mcx";
  const slab = document.createElement("div");
  slab.className = "mcx__slab";
  // The front last: in 3D the order does not matter, and if 3D is ever
  // painted flattened, the last face drawn is the one on top.
  for (const face of ["back", "left", "right", "top", "bottom", "front"]) {
    const side = document.createElement("div");
    side.className = `mcx__face mcx__face--${face}`;
    slab.append(side);
  }
  const front = slab.querySelector(".mcx__face--front");
  front.innerHTML = `
    <span class="mcx__grip"></span>
    <span class="mcx__label">
      <span class="mcx__stripe"></span>
      <span class="mcx__word">Memory<br />Card</span>
      <span class="mcx__size"></span>
    </span>
    <span class="mcx__arrow"></span>
    <span class="mcx__teeth"></span>`;
  card.append(slab);
  const shadow = document.createElement("div");
  shadow.className = "mcx__shadow";
  card.append(shadow);
  return card;
}

/**
 * The scene.
 *
 * `stage` is the element the camera looks into (it takes the pointer), `world`
 * holds everything in 3D, `track` holds the saves, and `card` is buildCard()'s.
 * `onPick(index)` is called when a save is clicked.
 */
export function createScene({ stage, world, track, card, onPick }) {
  let items = [];
  let selected = 0;
  let frame = 0;
  let last = 0;
  let scale = 1;

  /** Where the ring is turned to, in item positions (not degrees). */
  const ring = spring(0);
  /** The card's tilt towards the pointer, and its tip on the way in. */
  const tilt = { x: 0, y: 0, aimX: 0, aimY: 0 };
  /** 0 to 1 on the way in: the card is slotted, then the saves fan out of it. */
  const enter = spring(1);
  /** A nudge at an end of an arc that cannot go further. */
  const bump = spring(0);

  /** Degrees between neighbours, and whether the track closes into a ring. */
  function geometry() {
    const n = items.length;
    if (n < RING_FROM) return { step: STEP_ARC, closed: false };
    if (n * STEP_MIN > 360) return { step: STEP_MIN, closed: false };
    return { step: 360 / n, closed: true };
  }

  /** An item's place relative to the front, the short way round a ring. */
  function offset(index) {
    let rel = index - ring.x;
    const { closed } = geometry();
    const n = items.length;
    if (closed && n > 0) rel = ((((rel + n / 2) % n) + n) % n) - n / 2;
    return rel;
  }

  function layout() {
    const box = stage.getBoundingClientRect();
    // The scene is drawn for a 420 by 360 stage and scaled to the real one, so
    // a skin that makes the window bigger gets a bigger card, not a sparse one.
    scale = Math.max(0.6, Math.min(2.4, Math.min(box.width / 420, box.height / 360)));
    world.style.setProperty("--s", scale.toFixed(3));
  }

  function draw() {
    const { step } = geometry();
    const e = enter.x;
    // The saves follow the card in, a beat behind it.
    const fan = Math.max(0, Math.min(1, (e - 0.35) / 0.65));

    items.forEach((item, index) => {
      const rel = offset(index) + bump.x;
      const angle = rel * step * fan;
      const hidden = Math.abs(angle) > VISIBLE;
      item.style.visibility = hidden ? "hidden" : "";
      if (hidden) return;
      // How much this one is the chosen one: 1 at the front, 0 a place away.
      const focus = Math.max(0, 1 - Math.abs(rel));
      const fade = Math.max(0, Math.min(1, (VISIBLE - Math.abs(angle)) / (VISIBLE - FADE_FROM))) * fan;
      // Light falls off round the curve; the chosen one is fully lit.
      const shade = Math.min(0.66, (Math.abs(angle) / step) * 0.26) * (1 - focus);
      item.style.setProperty("--fade", fade.toFixed(3));
      item.style.setProperty("--shade", shade.toFixed(3));
      item.style.setProperty("--focus", focus.toFixed(3));
      item.style.zIndex = String(100 - Math.round(Math.abs(rel) * 4));
      if (flat) {
        item.style.transform = `translateX(${(rel * 118).toFixed(1)}px) translateY(22px) scale(${(0.82 + 0.22 * focus).toFixed(3)})`;
        return;
      }
      // Out to the track, facing out from the centre; neighbours turn back a
      // little towards you so their covers still read. The chosen one steps
      // forward, grows, lifts, and turns just off square so its spine shows,
      // then follows the pointer a few degrees.
      const radius = 150 * fan;
      item.style.transform =
        `rotateY(${angle.toFixed(2)}deg) translateZ(${radius.toFixed(1)}px) ` +
        `rotateY(${(-angle * 0.3).toFixed(2)}deg) ` +
        `translateZ(${(focus * 26).toFixed(1)}px) translateY(${(-focus * 8).toFixed(1)}px) ` +
        `rotateY(${(focus * (14 + tilt.x * 10)).toFixed(2)}deg) ` +
        `rotateX(${(focus * -tilt.y * 6).toFixed(2)}deg) ` +
        `scale(${(1 + focus * 0.12).toFixed(3)})`;
    });

    // The card drops into its slot, tips level, and leans after the ring as it
    // turns, so it reads as the thing the saves belong to.
    const lean = Math.max(-1, Math.min(1, ring.v * 0.12));
    card.style.setProperty("--fade", Math.min(1, e * 1.6).toFixed(3));
    card.style.transform = flat
      ? `translateY(${(-16 + (1 - e) * -40).toFixed(1)}px)`
      : `translateY(${((1 - e) * -150).toFixed(1)}px) ` +
        `rotateX(${((1 - e) * 32 + tilt.y * -8).toFixed(2)}deg) ` +
        `rotateY(${(-20 + tilt.x * 14 - lean * 9).toFixed(2)}deg)`;
  }

  function tick(now) {
    const dt = Math.min((now - last) / 1000, 1 / 30);
    last = now;
    stepSpring(ring, dt);
    stepSpring(enter, dt, 55, 12);
    stepSpring(bump, dt, 260, 18);
    const k = 1 - Math.exp(-dt * 7);
    tilt.x += (tilt.aimX - tilt.x) * k;
    tilt.y += (tilt.aimY - tilt.y) * k;
    draw();
    const resting =
      settled(ring) && settled(enter) && settled(bump) &&
      Math.abs(tilt.aimX - tilt.x) < 0.001 && Math.abs(tilt.aimY - tilt.y) < 0.001;
    if (resting) {
      frame = 0;
      return;
    }
    frame = requestAnimationFrame(tick);
  }

  /** Draw until everything has settled, then stop. */
  function wake() {
    if (still.matches) {
      snap(ring);
      snap(enter);
      snap(bump);
      tilt.x = tilt.aimX = 0;
      tilt.y = tilt.aimY = 0;
      draw();
      return;
    }
    if (frame) return;
    last = performance.now();
    frame = requestAnimationFrame(tick);
  }

  stage.addEventListener("pointermove", (event) => {
    if (still.matches) return;
    const box = stage.getBoundingClientRect();
    tilt.aimX = ((event.clientX - box.left) / box.width - 0.5) * 2;
    tilt.aimY = ((event.clientY - box.top) / box.height - 0.5) * 2;
    wake();
  });
  stage.addEventListener("pointerleave", () => {
    tilt.aimX = 0;
    tilt.aimY = 0;
    wake();
  });
  track.addEventListener("click", (event) => {
    const item = event.target.closest("[data-index]");
    if (item) onPick(Number(item.dataset.index));
  });

  const resize = new ResizeObserver(() => {
    layout();
    draw();
  });

  stage.classList.toggle("is-flat", flat);

  return {
    /** The track's elements, in order. Each carries its own `data-index`. */
    setItems(list) {
      items = list;
      // A ring keeps counting as it goes round; re-anchor it on the chosen
      // save without moving anything, so a list that changed length (a save
      // deleted) does not leave it aiming somewhere else.
      ring.x -= ring.to - selected;
      ring.to = selected;
      draw();
    },

    /** Whether moving past an end wraps round (a closed ring) or stops. */
    get closed() {
      return geometry().closed;
    },

    select(index, { instant = false } = {}) {
      const n = items.length;
      if (n && geometry().closed) {
        // The short way round: from the last to the first is one step on.
        let delta = index - selected;
        delta = ((((delta + n / 2) % n) + n) % n) - n / 2;
        ring.to += delta;
      } else {
        ring.to = index;
      }
      selected = index;
      if (instant) snap(ring);
      wake();
    },

    /** A push against an end of the arc: it gives, then springs back. */
    bump(direction) {
      bump.v -= direction * 3.2;
      wake();
    },

    /** The card goes into the slot and the saves come out of it. */
    enter() {
      enter.x = still.matches ? 1 : 0;
      enter.v = 0;
      enter.to = 1;
      wake();
    },

    start() {
      layout();
      resize.observe(stage);
      draw();
      wake();
    },

    stop() {
      resize.disconnect();
      cancelAnimationFrame(frame);
      frame = 0;
      tilt.aimX = tilt.aimY = tilt.x = tilt.y = 0;
    },
  };
}
