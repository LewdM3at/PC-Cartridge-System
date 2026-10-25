/**
 * The memory card view.
 *
 * The card itself in the middle of the window, as a piece of hardware, and
 * every save it carries as a game case on a track round it: the chosen one at
 * the front, stepped forward and lit, its neighbours turning away round the
 * curve. Under the stage, a panel says where that save stands and what can be
 * done with it. A drive with `memorycard.conf` opens straight into this; a
 * combo cartridge reaches it with a button or the pad's View, and goes back the
 * same way. memcard-scene.js does the 3D; this file decides what is chosen and
 * what the buttons do.
 *
 * Nothing here decides anything about the saves. Which copy is newer comes
 * from the save module's own comparison (`direction`), and every copy it asks
 * for is backed up by the backend first.
 *
 * Keys: left and right (or up and down) move, Enter copies the way the state
 * suggests, Delete asks to delete, O opens the save folder, Esc or M goes back.
 * Pad: A copies, Y asks to delete, B goes back. X does nothing here on purpose:
 * on the cartridge it ejects, and a button that ejects on one screen must not
 * delete on the next.
 */

import { buildCard, createScene } from "./memcard-scene.js";

const $ = (id) => document.getElementById(id);

/** How long an armed Delete waits for its second press. */
const ARMED_MS = 4000;

/** Only these get a lamp on the case: the saves that ask for something. */
function needsAttention(tone) {
  return tone === "warn" || tone === "bad";
}

/** What each state is called, and which way the obvious copy goes. */
function describe(block) {
  const onPc = block.hostBytes > 0;
  const onCard = block.cartridgeBytes > 0;
  switch (block.direction) {
    case "inSync":
      return { state: "Same on this PC and the card", tone: "good" };
    case "push":
      return onCard
        ? { state: "Newer on this PC", tone: "warn", suggest: "card" }
        : { state: "Only on this PC", tone: "muted", suggest: "card" };
    case "pull":
      return onPc
        ? { state: "Newer on the card", tone: "warn", suggest: "pc" }
        : { state: "Only on the card", tone: "muted", suggest: "pc" };
    case "conflict":
      return { state: "Changed on both", tone: "bad" };
    case "linked":
      return { state: "Played straight from the card", tone: "good" };
    case "unusable":
      return { state: block.detail || "Has no save folder on this PC", tone: "muted" };
    default:
      return { state: "Not played yet", tone: "muted" };
  }
}

export function createMemoryCard({ invoke, toast, formatBytes, duration, since }) {
  const el = {
    root: $("memcard"),
    title: $("mc-title"),
    capacity: $("mc-capacity"),
    barFill: $("mc-bar-fill"),
    cap: $("mc-cap"),
    stage: $("mc-scene"),
    world: $("mc-world"),
    blocks: $("mc-blocks"),
    empty: $("mc-empty"),
    prev: $("mc-prev"),
    next: $("mc-next"),
    count: $("mc-count"),
    panel: $("mc-panel"),
    game: $("mc-game"),
    state: $("mc-state"),
    facts: $("mc-facts"),
    toPc: $("mc-to-pc"),
    toCard: $("mc-to-card"),
    remove: $("mc-remove"),
    folder: $("mc-folder"),
    close: $("mc-close"),
  };
  const label = (button) => button.querySelector(".btn__label");
  const still = matchMedia("(prefers-reduced-motion: reduce)");

  let drivePath = "";
  let view = null;
  let selected = 0;
  let onClose = null;
  let busy = false;
  /** Delete asks twice: the first press arms it, the second does it. */
  let armed = false;
  let disarmTimer = null;
  /** True from opening until the first draw: the ring starts in place. */
  let opening = false;

  const card = buildCard();
  el.world.prepend(card);
  const scene = createScene({
    stage: el.stage,
    world: el.world,
    track: el.blocks,
    card,
    onPick: (index) => select(index),
  });

  function blocks() {
    return view?.blocks ?? [];
  }

  function disarm() {
    clearTimeout(disarmTimer);
    armed = false;
  }

  function render() {
    el.title.textContent = view.title;
    const count = blocks().length;
    el.root.classList.toggle("is-empty", count === 0);
    el.empty.hidden = count > 0;
    el.blocks.replaceChildren(
      ...blocks().map((block, index) => {
        const { state, tone } = describe(block);
        const li = document.createElement("li");
        li.className = "mc__block";
        li.setAttribute("role", "option");
        li.tabIndex = -1;
        li.dataset.index = String(index);
        li.dataset.tone = tone;
        li.setAttribute("aria-label", `${block.title}. ${state}.`);
        li.append(gameCase(block, tone));
        return li;
      }),
    );
    scene.setItems([...el.blocks.children]);
    select(Math.min(selected, Math.max(count - 1, 0)), { force: true, instant: opening });
    opening = false;
  }

  function select(index, { force = false, instant = false } = {}) {
    if (!force && index === selected) return;
    const changed = index !== selected;
    selected = index;
    disarm();
    const hadFocus = el.blocks.contains(document.activeElement);
    for (const li of el.blocks.children) {
      const on = Number(li.dataset.index) === index;
      li.setAttribute("aria-selected", String(on));
      // One stop in the tab order, on the save that is chosen.
      li.tabIndex = on ? 0 : -1;
      if (on && hadFocus) li.focus({ preventScroll: true });
    }
    scene.select(index, { instant });
    renderInfo();
    // The panel turns over to the new save; a refresh after a copy only
    // reprints it.
    if (changed && !still.matches) {
      el.panel.animate(
        [{ opacity: 0.25, transform: "translateY(5px)" }, { opacity: 1, transform: "none" }],
        { duration: 220, easing: "cubic-bezier(0.2, 0.8, 0.2, 1)" },
      );
    }
  }

  /** The panel under the stage: the chosen save, and what it needs. */
  function renderInfo() {
    const block = blocks()[selected];
    const count = blocks().length;
    el.root.classList.toggle("has-block", Boolean(block));
    el.count.textContent = count ? `${selected + 1} of ${count}` : "";
    const ends = !scene.closed;
    el.prev.disabled = count < 2 || (ends && selected === 0);
    el.next.disabled = count < 2 || (ends && selected === count - 1);
    if (!block) return;
    const { state, tone, suggest } = describe(block);
    el.game.textContent = block.title;

    const lamp = document.createElement("i");
    lamp.className = "lamp";
    lamp.dataset.tone = tone;
    lamp.setAttribute("aria-hidden", "true");
    el.state.replaceChildren(lamp, document.createTextNode(state));
    el.state.dataset.tone = tone;

    // Only what the drive and this PC actually know: no row for a figure that
    // is missing.
    const facts = [];
    const newest = Math.max(block.cartridgeNewest || 0, block.hostNewest || 0);
    if (newest) facts.push(`Saved ${since(newest)}`);
    if (block.seconds >= 60) {
      const launches = block.launches > 1 ? ` · ${block.launches} launches` : "";
      facts.push(`${duration(block.seconds)} played${launches}`);
    }
    if (block.cartridgeBytes) facts.push(`${formatBytes(block.cartridgeBytes)} on the card`);
    if (block.hostBytes) facts.push(`${formatBytes(block.hostBytes)} on this PC`);
    el.facts.replaceChildren(
      ...facts.map((text) => Object.assign(document.createElement("li"), { textContent: text })),
    );

    // Only the copy that fixes it. Both, unfilled, when both changed: the
    // choice is the player's, so neither is lit as the answer.
    const conflict = block.direction === "conflict";
    const hasFolder = Boolean(block.hostPath);
    el.toPc.hidden = !hasFolder || !(suggest === "pc" || conflict);
    el.toCard.hidden = !hasFolder || !(suggest === "card" || conflict);
    for (const button of [el.toPc, el.toCard]) {
      button.disabled = busy;
      button.classList.toggle("btn--primary", !conflict);
      button.classList.toggle("btn--ghost", conflict);
      button.classList.toggle("is-suggested", !conflict);
    }
    label(el.toPc).textContent = conflict ? "Keep the card's" : "Copy to PC";
    label(el.toCard).textContent = conflict ? "Keep this PC's" : "Copy to card";

    el.remove.hidden = !block.cartridgeBytes;
    el.remove.disabled = busy;
    // Nothing saved anywhere yet: there is no folder to open.
    el.folder.hidden = !block.hostBytes && !block.cartridgeBytes;
    el.remove.classList.toggle("is-armed", armed);
    label(el.remove).textContent = armed ? "Delete from card? A backup stays" : "Delete";
  }

  async function refresh() {
    try {
      view = await invoke("memory_card", { drivePath });
      el.empty.textContent = "No saves on this card yet.";
    } catch (error) {
      // Still a card on screen, saying what went wrong, rather than a window
      // that never opens.
      view = { title: view?.title ?? "Memory card", blocks: [] };
      el.empty.textContent = `This card could not be read: ${error}`;
    }
    render();
    void renderCapacity();
  }

  /** How full the card is: the saves, and what is left on the drive. */
  async function renderCapacity() {
    let health = null;
    try {
      health = await invoke("cartridge_health", { drivePath });
    } catch {
      // No reading, no bar: a guess would be worse.
    }
    const total = health?.totalBytes ?? 0;
    el.capacity.hidden = !total;
    // The card's moulded label carries its size, as a console card's did —
    // but only a size the drive reported.
    card.querySelector(".mcx__size").textContent = total ? formatBytes(total) : "";
    if (!total) return;
    const saves = blocks().reduce((sum, block) => sum + (block.cartridgeBytes || 0), 0);
    const used = total - (health.freeBytes ?? 0);
    el.barFill.style.transform = `scaleX(${Math.min(1, used / total).toFixed(3)})`;
    const held = saves ? `${formatBytes(saves)} of saves` : "No saves";
    el.cap.textContent = `${held} · ${formatBytes(health.freeBytes ?? 0)} free of ${formatBytes(total)}`;
  }

  async function copy(to) {
    const block = blocks()[selected];
    if (!block || busy) return;
    busy = true;
    renderInfo();
    try {
      await invoke("memcard_copy", { drivePath, slotId: block.slot.id, to });
      toast(`${block.title} copied to ${to === "pc" ? "this PC" : "the card"}. The copy it replaced was kept.`);
    } catch (error) {
      toast(`${block.title} was not copied: ${error}`, true);
    } finally {
      busy = false;
      await refresh();
    }
  }

  async function remove() {
    const block = blocks()[selected];
    if (!block || busy || !block.cartridgeBytes) return;
    if (!armed) {
      armed = true;
      clearTimeout(disarmTimer);
      disarmTimer = setTimeout(() => {
        armed = false;
        renderInfo();
      }, ARMED_MS);
      renderInfo();
      return;
    }
    disarm();
    busy = true;
    try {
      await invoke("memcard_remove", { drivePath, slotId: block.slot.id });
      toast(`${block.title} taken off the card. A backup stays on it, and this PC's copy is untouched.`);
    } catch (error) {
      toast(`${block.title} was not deleted: ${error}`, true);
    } finally {
      busy = false;
      await refresh();
    }
  }

  async function reveal() {
    const block = blocks()[selected];
    if (!block) return;
    try {
      await invoke("memcard_reveal", { drivePath, slotId: block.slot.id });
    } catch (error) {
      toast(String(error), true);
    }
  }

  /** The copy the state suggests, for Enter and the pad's A. */
  function primary() {
    const block = blocks()[selected];
    if (!block) return;
    const { suggest, tone } = describe(block);
    if (suggest) {
      void copy(suggest);
      return;
    }
    // Nothing to do is still an answer, and a silent press reads as broken.
    toast(
      tone === "bad"
        ? `${block.title} changed on both sides. Choose Copy to PC or Copy to card.`
        : `${block.title} needs nothing: ${describe(block).state.toLowerCase()}.`,
    );
  }

  /**
   * One step along the track. The track is one line, so both axes move along
   * it, as the launcher's rail does. A ring wraps; an arc stops at its ends and
   * gives a little, so a held direction cannot run off the end unnoticed.
   */
  function move(x, y) {
    const count = blocks().length;
    const step = Math.sign(x || y);
    if (count < 2 || !step) return;
    const next = selected + step;
    if (scene.closed) {
      select((next + count) % count);
    } else if (next < 0 || next >= count) {
      scene.bump(step);
    } else {
      select(next);
    }
  }

  // A wheel or a trackpad moves one save per notch, not per pixel.
  let wheel = 0;
  let wheelAt = 0;
  el.stage.addEventListener(
    "wheel",
    (event) => {
      event.preventDefault();
      wheel += Math.abs(event.deltaX) > Math.abs(event.deltaY) ? event.deltaX : event.deltaY;
      const now = performance.now();
      if (Math.abs(wheel) < 40 || now - wheelAt < 130) return;
      move(Math.sign(wheel), 0);
      wheel = 0;
      wheelAt = now;
    },
    { passive: false },
  );

  el.prev.addEventListener("click", () => move(-1, 0));
  el.next.addEventListener("click", () => move(1, 0));
  el.toPc.addEventListener("click", () => copy("pc"));
  el.toCard.addEventListener("click", () => copy("card"));
  el.remove.addEventListener("click", remove);
  el.folder.addEventListener("click", reveal);
  el.close.addEventListener("click", () => onClose?.());

  return {
    get open() {
      return !el.root.hidden;
    },

    /**
     * Show a drive's card. `close` is what leaving it means here, and `back`
     * says whether that is returning to a cartridge rather than closing the
     * window, which is what the close button is then called.
     */
    async show(path, close, { back = false } = {}) {
      drivePath = path;
      onClose = close;
      selected = 0;
      opening = true;
      const name = back ? "Back to cartridge" : "Close";
      el.close.setAttribute("aria-label", name);
      el.close.title = `${name} — Esc`;
      await refresh();
      el.root.hidden = false;
      scene.start();
      scene.enter();
    },

    hide() {
      disarm();
      scene.stop();
      el.root.hidden = true;
    },

    /** A key while the card is open. True when it was the card's. */
    key(event) {
      const arrows = { ArrowLeft: [-1, 0], ArrowRight: [1, 0], ArrowUp: [0, -1], ArrowDown: [0, 1] };
      // Enter on a focused button is that button's; only elsewhere is it the
      // suggested copy.
      if (event.key === "Enter" && event.target instanceof HTMLButtonElement) return false;
      if (arrows[event.key]) {
        move(...arrows[event.key]);
      } else if (event.key === "Enter") {
        primary();
      } else if (event.key === "Delete") {
        void remove();
      } else if (event.key === "o" || event.key === "O") {
        void reveal();
      } else if (event.key === "Escape" || event.key === "m" || event.key === "M") {
        onClose?.();
      } else {
        return false;
      }
      event.preventDefault();
      return true;
    },

    pad: { move, primary, remove: () => void remove(), back: () => onClose?.() },
  };
}

/**
 * A save as a game case: the art on the cover, the game's name running down
 * the spine, lids top and bottom, a lamp on the corner when it needs something,
 * and the shadow it stands in. A game with no art gets a cover in the card's
 * accent with its initials.
 */
function gameCase(block, tone) {
  const wrap = document.createElement("span");
  wrap.className = "mc__case";
  const art = block.icon && /^(data:image\/|src\/)/.test(block.icon) ? block.icon : "";
  // Absolute, because a url() inside a custom property resolves against the
  // stylesheet that uses it, not the page — which breaks a relative path.
  if (art) wrap.style.setProperty("--art", `url("${new URL(art, document.baseURI).href}")`);
  else wrap.classList.add("is-blank");
  // The front last: if 3D is ever painted flattened, it is the face on top.
  for (const face of ["back", "right", "left", "top", "bottom", "front"]) {
    const side = document.createElement("span");
    side.className = `mc__face mc__face--${face}`;
    if (!art && face === "front") side.textContent = initials(block.title);
    if (face === "left") {
      const spine = document.createElement("span");
      spine.textContent = block.title;
      side.append(spine);
    }
    if (face === "front" && needsAttention(tone)) {
      const lamp = document.createElement("i");
      lamp.className = "lamp mc__case-lamp";
      lamp.dataset.tone = tone;
      lamp.setAttribute("aria-hidden", "true");
      side.append(lamp);
    }
    wrap.append(side);
  }
  const shadow = document.createElement("span");
  shadow.className = "mc__shadow";
  wrap.append(shadow);
  return wrap;
}

function initials(title) {
  return (title || "")
    .split(/\s+/)
    .filter((word) => /[\p{L}\p{N}]/u.test(word))
    .slice(0, 2)
    .map((word) => [...word][0].toUpperCase())
    .join("");
}
