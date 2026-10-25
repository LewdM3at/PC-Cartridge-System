/**
 * Gamepad navigation for the launcher.
 *
 * A cartridge on a shelf next to a TV wants a controller, not a mouse. The
 * webview already exposes the Gamepad API, so this needs nothing from the Rust
 * side and nothing resident: the polling loop only exists while a pad is
 * connected and the window is open, which is a few seconds per insert.
 *
 * Every action has its own button, and the stick only ever changes which game
 * is selected:
 *
 *   D-pad / left stick   move through the game list, on both axes
 *   A (south)            Play
 *   X (west)             Eject
 *   Y (north)            Details
 *   B (east)             back out of details, or dismiss the window
 *   View / Select        the memory card, on a combo drive
 *   Start                Play
 *
 * The window used to move focus between its controls instead, so reaching Eject
 * meant pressing down past Play and the rail. That is how a form works, not how
 * a thing plugged into a television works: on a console the buttons *are* the
 * actions, and up and down are for choosing. Four controls fit on four face
 * buttons exactly, so nothing has to be navigated to.
 *
 * Nothing here launches anything by itself — each button calls the same
 * function the mouse and the keyboard call, so a controller cannot do more than
 * a person at the keyboard could.
 *
 * The prompts follow the device in your hands, not the device plugged in. A pad
 * connected and sitting on the table used to be enough to replace every keycap
 * with a face-button icon and leave it that way, so a PC with a controller
 * attached — or a Steam Deck in desktop mode with a keyboard — showed the wrong
 * prompt to whoever was actually typing. Now the icons appear when a pad button
 * is pressed and the keycaps come back on the next keystroke. The idea is
 * ArtMoon's and StreamLight's, both gamepad-first Moonlight forks; see
 * `docs/STATUS.md`.
 */

/** Standard-layout indices, named so the mapping below reads as English. */
const BUTTON = { SOUTH: 0, EAST: 1, WEST: 2, NORTH: 3, SELECT: 8, START: 9, UP: 12, DOWN: 13, LEFT: 14, RIGHT: 15 };

/** Sticks rest slightly off centre; below this a pad is being held, not moved. */
const DEADZONE = 0.55;

/** How long a held direction waits before repeating, then how fast. */
const REPEAT_DELAY = 420;
const REPEAT_RATE = 140;

/**
 * Turn a snapshot of a pad into the buttons that went down this frame.
 *
 * Edge detection, not level: holding A must not press the button sixty times a
 * second. Directions repeat on a timer instead, which is what a menu should do.
 */
export function pressed(previous, current) {
  const down = [];
  for (let i = 0; i < current.length; i += 1) {
    if (current[i] && !previous[i]) down.push(i);
  }
  return down;
}

/**
 * The pads worth listening to.
 *
 * Not simply the first one connected, which is what this used to take. Plenty
 * of things that are not controllers enumerate as gamepads — a Keychron
 * keyboard's own dongle does, reporting sixteen buttons that are never pressed
 * and an empty `mapping` — and if one of those sorts ahead of the real pad, the
 * launcher polls it forever and every button on the actual controller does
 * nothing. That is a silent failure: the pad raises `gamepadconnected` and then
 * nothing works. It is quieter now than it was, since the prompts no longer
 * change until a button is actually pressed — so the only outward sign is a
 * controller that does nothing at all, which is what the log below is for.
 *
 * So: anything reporting the standard layout, if anything does. Only when
 * nothing does is everything considered, which keeps an unusual but real pad
 * working rather than trading one exclusion for another.
 */
export function pads() {
  const all = [...(navigator.getGamepads?.() ?? [])].filter(Boolean);
  const standard = all.filter((pad) => pad.mapping === "standard");
  return standard.length > 0 ? standard : all;
}

/**
 * One state from several pads: a button is down if it is down on any of them.
 *
 * Two controllers plugged into a machine by a television is normal, and which
 * one somebody picks up is not worth having an opinion about.
 */
export function merge(list) {
  const state = [];
  for (const pad of list) {
    const one = snapshot(pad);
    for (let i = 0; i < one.length; i += 1) state[i] = state[i] || one[i];
  }
  return state;
}

/** A pad's buttons and sticks, flattened into one array of booleans. */
export function snapshot(pad) {
  const state = pad.buttons.map((b) => b.pressed || b.value > 0.5);
  const [x = 0, y = 0] = pad.axes;
  // The stick is folded into the d-pad, so the rest of the code has one thing
  // to think about.
  state[BUTTON.UP] = state[BUTTON.UP] || y < -DEADZONE;
  state[BUTTON.DOWN] = state[BUTTON.DOWN] || y > DEADZONE;
  state[BUTTON.LEFT] = state[BUTTON.LEFT] || x < -DEADZONE;
  state[BUTTON.RIGHT] = state[BUTTON.RIGHT] || x > DEADZONE;
  return state;
}

/**
 * Start listening for controllers.
 *
 * `actions` supplies what the launcher wants done, so this file stays about
 * pads and knows nothing about cartridges.
 */
export function connect(actions) {
  let previous = [];
  let running = false;
  let held = null;
  let heldSince = 0;
  let lastRepeat = 0;

  // Diagnostics. A pad that reports as connected and then reads as though every
  // button were resting looks identical, from in here, to a pad nobody is
  // touching — so the only way to tell them apart is to write down what each
  // frame actually saw.
  const log = actions.log ?? (() => {});
  let frames = 0;
  let lastReport = 0;

  function move([x, y]) {
    actions.move(x, y);
  }

  function frame(now) {
    if (!running) return;
    frames += 1;

    const list = pads();

    // Once a second, and only while something is listening.
    if (now - lastReport > 1000) {
      lastReport = now;
      log(
        `pads=[${list.map((p) => p.id)}] frames=${frames} focus=${document.hasFocus()}`,
      );
      frames = 0;
    }

    if (list.length > 0) {
      const state = merge(list);

      const down = pressed(previous, state);
      if (down.length > 0) {
        log(`pressed ${down}`);
        usingPad(true);
      }
      for (const button of down) {
        switch (button) {
          case BUTTON.SOUTH:
          case BUTTON.START:
            actions.play();
            break;
          case BUTTON.WEST:
            actions.eject();
            break;
          case BUTTON.NORTH:
            actions.details();
            break;
          case BUTTON.EAST:
            actions.back();
            break;
          case BUTTON.SELECT:
            actions.card?.();
            break;
          case BUTTON.UP:
          case BUTTON.DOWN:
          case BUTTON.LEFT:
          case BUTTON.RIGHT: {
            // Both axes are reported, and the launcher decides what each means
            // for the layout it is currently wearing: down the column in a
            // list, along the strip in a horizontal one, by a row in a grid.
            const direction = {
              [BUTTON.UP]: [0, -1],
              [BUTTON.DOWN]: [0, 1],
              [BUTTON.LEFT]: [-1, 0],
              [BUTTON.RIGHT]: [1, 0],
            }[button];
            move(direction);
            held = { button, direction };
            heldSince = now;
            lastRepeat = now;
            break;
          }
        }
      }

      // A held direction keeps moving, after a pause long enough that a single
      // press never repeats by accident.
      if (!held || !state[held.button]) {
        held = null;
      } else if (now - heldSince > REPEAT_DELAY && now - lastRepeat > REPEAT_RATE) {
        move(held.direction);
        lastRepeat = now;
      }

      previous = state;
    }

    requestAnimationFrame(frame);
  }

  /**
   * Say which device is being held, and redraw the prompts if it changed.
   *
   * The focus ring rides along: it is normally only drawn for keyboard users,
   * and with a pad in hand it is the only way to see where you are.
   */
  function usingPad(pad) {
    if (document.body.classList.contains("is-gamepad") === pad) return;
    document.body.classList.toggle("is-gamepad", pad);
    log(`prompts now ${pad ? "pad" : "keyboard"}`);
    actions.changed?.();
  }

  function start() {
    if (running) return;
    running = true;
    // Polling begins as soon as a pad is there, so the first button press is
    // acted on — but the prompts do not change until that press happens. A
    // controller plugged in and left alone is not the device in anyone's hands.
    requestAnimationFrame(frame);
  }

  function stop() {
    running = false;
    previous = [];
    held = null;
    usingPad(false);
  }

  // Back to keycaps the moment somebody types. Capturing, so it is seen whatever
  // the launcher's own key handling does with the event, and passive because
  // this only ever reads it.
  window.addEventListener("keydown", () => usingPad(false), {
    capture: true,
    passive: true,
  });

  window.addEventListener("gamepadconnected", (event) => {
    log(
      `gamepadconnected index=${event.gamepad?.index} id=${event.gamepad?.id} ` +
        `mapping=${event.gamepad?.mapping} buttons=${event.gamepad?.buttons?.length}`,
    );
    start();
  });
  window.addEventListener("gamepaddisconnected", () => {
    // Only stop once the last pad has gone.
    const any = [...(navigator.getGamepads?.() ?? [])].some(Boolean);
    if (!any) stop();
  });

  // A pad that was already connected when the window opened raises no event
  // until a button is pressed, so look once at startup.
  if ([...(navigator.getGamepads?.() ?? [])].some(Boolean)) start();

  return { start, stop };
}
