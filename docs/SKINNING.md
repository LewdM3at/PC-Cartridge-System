# Writing a skin

A skin is one stylesheet. It is loaded **on top of** `style.css`, never instead
of it, so a skin file is only the difference — the shipped ones run from 150 to
280 lines and none of them touches the markup.

There is no JavaScript in a skin, no template, and no build step. Everything on
this page is a CSS selector or a custom property.

---

## Where it goes

```
<drive>\.gamepak\skin.css
```

One file, on the cartridge, beside the artwork. Replug the drive and the
launcher wears it. There is no name to register and no `skin=` key: a cartridge
either carries a stylesheet or it does not.

It is read by the backend, capped at 256 KB, and inlined into the window as
text — the window never opens a path on the drive, which is the same arrangement
the artwork already had.

Fourteen worked examples are in [`skins/`](skins/). Copy one and edit it.

**What this trades away.** A stylesheet cannot run code, but it can move, cover
and restyle anything on screen, including making Eject look like Play or putting
a convincing false sentence where a real one was. This is for cartridges you
wrote yourself: a fair trade on your own shelf, a poor one for a drive somebody
handed you.

---

## What a skin decides

Three custom properties on `:root`. All optional; saying nothing keeps the stock
behaviour, so a skin that does not care pays nothing.

```css
:root {
  --skin-width: 560;       /* logical px. Default 420 */
  --skin-height: 430;      /* logical px. Default 630 */

  --skin-art: hero;        /* the fill behind everything */
  --skin-row-art: grid;    /* each row's thumbnail */
  --skin-logo: game;       /* the logo printed over the fill */
}
```

**Size.** The window resizes itself to match once the stylesheet has loaded.
420×630 is the shape of a cover, which is right for a skin built around one and
wrong for a skin laid out sideways.

**Artwork — three slots, not one.** A window showing a picture in three places
wants three different pictures in them: a hero behind everything, covers in the
rail, and the selected game's logo printed in front. That is Big Picture, and
with a single property a skin could only ask for the same picture three times.

| property | drives | takes | default |
|---|---|---|---|
| `--skin-art` | the fill behind everything | `grid` `hero` `logo` `icon` `none` | `grid` |
| `--skin-row-art` | each row's thumbnail | the same | falls back to `--skin-art` |
| `--skin-logo` | the printed logo | `auto` `game` `none` | `auto` |

A slot left unset falls back to `--skin-art`, so a skin that wants one kind
throughout still says it once. Anything unrecognised gets the grid. `none` means
no picture at all — worth saying for a rail of names with no thumbnails, which
would otherwise build an image element per row to show nothing.

Every game carries all four kinds and any may be empty, so the fill falls
through: the game's picture of the kind you asked for, then the cartridge's,
then the game's cover, then the cartridge's, then the game's initials. A row is
that game, so it takes that game's picture and stops — otherwise a grid of ten
shortcuts is ten copies of one cover.

`--skin-logo: auto` is the stock behaviour: a game prints its own logo instead
of the heading — on a collection the **selected game's**, following the rail as
it moves, and a game with no logo keeps the heading. The collection's own logo
names the collection, not the pick, so it is never printed over a game; it goes
to `#cart-mark` in the corner. `game` is the same except that a game with no logo
borrows the collection's — pair it with `--skin-art: hero` and
`--skin-row-art: grid` and you have a couch launcher. `none` keeps the heading
as type. Whenever a logo is printed, `#stage` gains `.has-logo` and the
base hides `#game-title`, so nothing repeats the name.

`icon` is really `autorun.inf`'s and is 256px, so it makes a poor fill. It is on
the list because a skin maker with a reason is better served by having it.

---

**The unboxing.** The first time a PC sees a cartridge, the launcher opens it
like a new game: `#unbox` shows the box (`.unbox__lid` with the cover as
`.unbox__front` and the shrink-wrap gloss as `.unbox__wrap`), the lid swings
open, and `.unbox__cart` — ridges, then a `.unbox__label` carrying the art, the
logo or the title — lifts out before the cartridge seats. It plays once per
cartridge per machine; any key, click or pad button skips it. A skin can restyle
any of those parts, or turn it off:

```css
:root { --skin-unbox: none; }
```

## The markup a skin styles

This never changes. A skin cannot add elements, which is what the two owned
layers below are for.

```
#card                        the window
├── #slot                    what is behind the cartridge; seen after Eject
│   ├── #slot-label
│   └── #btn-insert
└── #face                    everything printed on the cartridge — not just
                             the artwork. #stage is inside it, so insetting
                             #face clips the whole interface, not the picture.
    ├── #plate               the artwork
    │   ├── #cover-img       two layers, so a collection can cross-fade
    │   ├── #cover-img-b
    │   └── #cover-placeholder   shown when there is no picture
    │       ├── #placeholder-initials
    │       └── #placeholder-name
    ├── #skin-chrome         yours. Over the artwork
    ├── #skin-panel          yours. Behind the controls
    ├── #scrim-top           gradients, so type does not depend on the art
    ├── #scrim-bottom
    ├── #cart-mark           a collection's logo, in the corner
    ├── #chrome              the three corner buttons
    │   └── #chrome-actions
    │       ├── #btn-details     ⓘ  carries .pad-badge
    │       └── #btn-close       ✕
    ├── #stage
    │   ├── #eyebrow
    │   ├── #game-list          the rail. One .game-row per game
    │   ├── #title-logo         printed instead of the title when there is one
    │   ├── #game-title
    │   ├── #notice             why Play is disabled, when it is
    │   └── #button-row
    │       ├── #play-ring > #btn-play    .btn.btn--play — the ring is the
    │       │                            flex child and is flex: 1, so set
    │       │                            #play-ring, not .btn--play, to size it
    │       └── #btn-eject                .btn.btn--eject, carries .pad-badge
    ├── #sheet                  the ⓘ panel
    └── #toast
```

A row in `#game-list`, built at runtime:

```
.game-row[aria-selected]
├── .game-row__cover              <img>, or a <span class="game-row__cover--empty">
│                                 holding the game's initials when it has no picture
├── .game-row__body
│   ├── .game-row__title
│   └── .game-row__meta           empty in the stock look; the row's data-size carries the band
└── .game-row__dot                lit for the selected row
```

### The two layers that are yours

Empty in the stock look, and nothing in them draws itself. Fill them with
pseudo-elements — this is how a skin adds a marquee, a speaker grille, a title
bar or a power lamp without any markup.

```css
#skin-chrome::before { /* over the artwork */ }
#skin-panel::after   { /* behind the controls */ }
```

Both ignore the pointer, so nothing a skin draws can get between somebody and a
button. Do not undo that.

---

## States to style

| Selector | When |
|---|---|
| `.game-row[aria-selected="true"]` | the game Play will start |
| `.game-row:focus` | the same row — focus follows the selection |
| `.btn:disabled` | no executable, or a job is running |
| `.btn--play.is-launching` | between the press and the game appearing |
| `#card.is-collection` | more than one game, so the rail is showing |
| `#card.is-ejected` | the face has ridden out of the slot |
| `#card.is-crossfaded` | the second art layer is in front |
| `#stage.has-logo` | a logo is printed instead of the title |
| `#game-title.is-long` | the name runs over fifteen characters |
| `.game-row[data-size]` | `large` 40 GB+, `medium` 12 GB+, `small`, `none` |
| `body.is-gamepad` | a controller is connected |
| `.hidden`, `[hidden]` | do not make these visible |

`data-size` is the one hook a layout can key on. The size is printed on the row
as text, which a skin can style but cannot lay out from — CSS has no way to read
"64.2 GB" and turn it into a column span. Bands rather than the number, because
a skin wants three or four cases and not a scale:

```css
.game-row[data-size="large"]  { grid-column: span 2 }   /* shelf */
.game-row[data-size="large"]  { flex-basis: 268px }     /* eurorack */
```

`is-long` sets no properties of its own. The base reads two variables deep —
`font-size: var(--title-size, var(--auto-title-size, 40px))` — where the outer
one is yours and the base never sets it, and the inner one is the squeeze, which
only fills in for a skin that said nothing:

```
--title-size  --title-stretch  --title-tracking  --title-leading
```

Set them if you want a title size of your own, and leave them alone to inherit
the squeeze. Setting `font-size` on `#game-title` directly works too.

**The rule behind that shape:** a state on `#card` or `#stage` must never set a
property a skin might want. `#card.is-collection #scrim-bottom` is two ids and a
class, so it beat every skin's `#scrim-bottom` and blacked out the whole window
the moment a cartridge held more than one game. Both that and the long-title
squeeze now set variables the one-id rule reads. If you find a state that still
wins against you, that is a bug in the base, not something to out-specify.

---

## Play stats

What the cartridge has recorded about each game — playtime, launches, when it
was last played, and HowLongToBeat's estimate if the cartridge was written with
one — lives in `cartridge.conf` (see the [manual](MANUAL.md#hours-and-saves-that-travel)).
A skin cannot read files, so the launcher reads it and hands it over three ways.

**Custom properties**, on `#card` for the game Play will start and on each
`.game-row` for its own game. Strings are quoted, for `content:`; the `-hours`
and `--progress` values are bare numbers, for `calc()`:

| Property | Example | |
|---|---|---|
| `--playtime` | `"13 h 12 min"` | empty under a minute |
| `--playtime-hours` | `13.20` | |
| `--launches` | `9` | a number: `counter-reset: n var(--launches)` prints it |
| `--last-played` | `"2 days ago"` | empty if never |
| `--hltb-main` `--hltb-extra` `--hltb-complete` | `"21 h"` | empty without an estimate |
| `--hltb-main-hours` | `21.00` | `0` without one |
| `--progress` | `0.629` | playtime ÷ main story, capped at 1; `0` without an estimate |

**Data attributes**, on the same elements, for a skin that wants cases rather
than a scale:

| Attribute | Values |
|---|---|
| `data-progress` | `new` (never played), `unknown` (no estimate), `started` under half, `halfway` under 90%, `nearly`, `beaten` |
| `data-played` | `yes` `no` |
| `data-hltb` | `yes` `no` |

**What the cartridge is for.** `#card` carries `data-platform`, the
cartridge's `platform=` in lower case — `pc` when it does not say. A skin that
dresses a Super Nintendo cartridge differently from a PC game box keys on it:

```css
#card[data-platform="snes"] #plate { border-radius: 4px 4px 18px 18px }
```

The names are listed in `cartridge.conf.example`.

**A stats line**, `#play-stats`, under the title — hidden in the stock window,
because the details sheet already says all of it. Show it and style it:

```css
#play-stats { display: flex }             /* 13 h 12 min  of ~21 h  · 2 days ago */
.play-stats__fill { background: gold }    /* the progress bar, sized by --progress */
.game-row::after { content: var(--playtime) }        /* each row's own hours */
.game-row[data-progress="beaten"] .game-row__title::after { content: " ✓" }
```

Its parts are `.play-stats__time`, `.play-stats__beat`, `.play-stats__last` and
`.play-stats__bar` holding `.play-stats__fill`. The bar hides itself when there
is no estimate.

---

## Tokens worth overriding

Defined in `tokens.css`. Redefine on `:root`.

```
--accent  --accent-ink        the one accent, normally sampled from the cover
--radius                      the window's corner
--shell --panel --sunk        surfaces
--ink --ink-2 --ink-3         text
--hairline --hairline-soft    borders
--danger --good               states
--ease                        the shared easing curve
```

`--accent` is re-sampled from each cover at load. A skin whose palette is painted
rather than printed — a CRT's front panel, a HUD — should set it and mean it.

---

## What is not allowed

**No network.** The content security policy is `default-src 'self'`, with images
also permitted from `data:` and steamgriddb.com. So:

- `@import url('https://fonts.googleapis.com/...')` silently loads nothing.
  Bundle a `woff2` next to the two already in `src/fonts/` and `@font-face` it,
  or use `"Archivo"` and `"Spline Sans Mono"`, which are already there.
- No remote background images.

**Watch the specificity.** Parts of the base stylesheet are id-scoped —
`#chrome .icon-btn`, `#card.is-collection #scrim-bottom` — and a bare class
selector loses to those and silently does nothing. If a rule appears to be
ignored, that is the first thing to check.

**No markup, no script.** A skin is a stylesheet. If you find yourself needing an
element, use `#skin-chrome` or `#skin-panel`.

**Nothing that makes the launcher unusable.** These are not enforced, and they
are the whole reason a skin is CSS and not a program:

- Do not hide or cover `#btn-play` or `#btn-eject`. Nothing protects those two:
  they are inside `#stage`, which a skin lays out.
- `#chrome` — the ⓘ and ✕ — sits at `z-index: 6`, above `#stage`, both owned
  layers, and any overlay on `#face`. So positioning `#stage` absolutely is
  safe, which several skins do. Do not lower it, and do not draw an opaque
  layer over that corner: it would still be clickable and no longer visible,
  which is worse than either.
- Keep a visible focus state. `body.is-gamepad` exists so a controller user can
  see where they are.
- Keep `:disabled` looking disabled.
- Anything that moves should stop under `@media (prefers-reduced-motion: reduce)`.

---

## A minimal skin

```css
:root {
  --skin-width: 480;
  --skin-height: 640;
  --skin-art: hero;
  --accent: oklch(0.7 0.15 300);
  --accent-ink: oklch(0.15 0.03 300);
}

#card { background: oklch(0.1 0.02 300); border-radius: 4px; }

.btn--play { border-radius: 2px; letter-spacing: 0.2em; }

#skin-chrome::before {
  content: "";
  position: absolute;
  inset: 0 0 auto 0;
  height: 3px;
  background: var(--accent);
}
```

Save that as `.gamepak/skin.css` on a cartridge and replug it.

---

## Seeing it without a cartridge

The launcher is plain HTML and falls back to a preview harness when there is no
Tauri behind it, so a browser will run it:

```bash
python -m http.server 8731        # from the repository root
```

- `localhost:8731/tauri-ui/app/index.html?drive=D:\` — one game
- `…&state=bundle` — a collection
- `…&state=noexec` — a cartridge with nothing to play
- `…&skin=retro` — hands the launcher `docs/skins/retro.css` as if the cartridge
  were carrying it, which is the same path a real one takes

Serve the repository root, not `tauri-ui/app`, or `&skin=` cannot reach
`docs/skins/`.

The browser window will not be 420x630, and a skin that sets its own size cannot
resize a tab, so put it in an iframe of the right size to judge proportions:

```html
<iframe src="index.html?drive=D:\&state=bundle" style="width:420px;height:630px;border:0"></iframe>
```

The content security policy is Tauri's, not the server's, so a remote `@import`
that works here will still load nothing in the real window. Check that against
the list above rather than against the preview.

## When it does not work

```
PC_GAMEPAK_DEBUG=1
```

The launcher then writes to `%LOCALAPPDATA%\PC-GamePak\launcher.log`, beside the
watcher's, and says which skin it was asked for, whether the stylesheet was
attached, and what the window resized to. A release build has no devtools and the
window closes on eject, so this is the only way to see inside it.

The examples in [`skins/`](skins/) are the other reference, and they are worth
reading in the order they diverge from the stock window: `cozy` recolours it,
`luna` and `bricolage` rebuild it as a document, `akihabara` and `phantom` move
the furniture, and `cyberpunk` changes what shape the list is.
