---
name: PC GamePak
description: Removable drives as game cartridges — a slot, a cover, one press.
colors:
  accent: "oklch(0.72 0.15 68)"
  accent-ink: "oklch(0.16 0.02 65)"
  shell: "oklch(0.155 0.005 65)"
  panel: "oklch(0.185 0.006 65)"
  sunk: "oklch(0.125 0.005 65)"
  ground: "oklch(0.115 0 0)"
  ink: "oklch(0.97 0.006 75)"
  ink-soft: "oklch(0.86 0.008 75)"
  ink-2: "oklch(0.75 0.008 75)"
  ink-3: "oklch(0.635 0.008 75)"
  hairline: "oklch(1 0 0 / 0.16)"
  hairline-mid: "oklch(1 0 0 / 0.11)"
  hairline-soft: "oklch(1 0 0 / 0.075)"
  lift: "oklch(1 0 0 / 0.04)"
  well: "oklch(0 0 0 / 0.26)"
  danger: "oklch(0.68 0.19 27)"
  good: "oklch(0.76 0.15 145)"
typography:
  display:
    fontFamily: "Archivo, ui-sans-serif, sans-serif"
    fontSize: "40px"
    fontWeight: 800
    lineHeight: 1
    letterSpacing: "-0.01em"
  headline:
    fontFamily: "Archivo, ui-sans-serif, sans-serif"
    fontSize: "19px"
    fontWeight: 800
    lineHeight: 1.15
  title:
    fontFamily: "Archivo, ui-sans-serif, sans-serif"
    fontSize: "14px"
    fontWeight: 700
    lineHeight: 1.2
  body:
    fontFamily: "Archivo, ui-sans-serif, sans-serif"
    fontSize: "12.5px"
    fontWeight: 400
    lineHeight: 1.45
  label:
    fontFamily: "Archivo, ui-sans-serif, sans-serif"
    fontSize: "11px"
    fontWeight: 700
    letterSpacing: "0.19em"
  data:
    fontFamily: "Spline Sans Mono, ui-monospace, monospace"
    fontSize: "12px"
    fontWeight: 400
    fontFeature: "tnum"
rounded:
  control: "5px"
  card: "11px"
  pill: "999px"
spacing:
  xs: "4px"
  sm: "8px"
  md: "14px"
  lg: "20px"
components:
  button-primary:
    backgroundColor: "{colors.accent}"
    textColor: "{colors.accent-ink}"
    typography: "{typography.label}"
    rounded: "{rounded.control}"
    height: "46px"
    padding: "0 20px"
  button-ghost:
    backgroundColor: "{colors.lift}"
    textColor: "{colors.ink}"
    typography: "{typography.label}"
    rounded: "{rounded.control}"
    height: "46px"
    padding: "0 17px"
  icon-button:
    backgroundColor: "{colors.well}"
    textColor: "{colors.ink-2}"
    rounded: "{rounded.control}"
    size: "34px"
  build-card:
    backgroundColor: "{colors.well}"
    textColor: "{colors.ink}"
    rounded: "{rounded.card}"
    padding: "10px 12px"
  save-block:
    backgroundColor: "{colors.lift}"
    textColor: "{colors.ink-2}"
    rounded: "{rounded.control}"
    padding: "14px 8px 10px"
---

# Design System: PC GamePak

## Overview

**Creative North Star: "The Slot and the Cartridge"**

The launcher window is a console's cartridge slot and the cover art is the cartridge seated in it. Everything the system does comes from that one image: the cover fills the window, the chrome prints on top of it and gets out of its way, Eject rides the whole face out of the slot and leaves the empty slot behind, and a new cartridge comes out of its box the first time a machine sees it. The wizard is the desk where cartridges are made, in the same warm dark and the same type, with a live launcher beside the form so every choice is seen where it will land.

It is read from a couch as often as from a desk, and driven by a gamepad as often as a mouse. So the system is quiet, warm and large where it matters: one filled button per screen, type that holds from across a room, and state said in words rather than colour alone. The single accent is not the brand's — it is sampled from each cover at load, so Play belongs to whatever game is in the slot.

**Key Characteristics:**
- Cover art is the surface; chrome is a thin print over it.
- One accent, sampled from the cartridge's own art, contrast-checked for its ink.
- Warm near-black neutrals (hue 65–75), never cold grey.
- Physical motion that means something: seat, eject, unbox. Nothing ambient.
- Four actions on four face buttons; every control reachable by pad and keys.

## Colors

A warm dark ground with one borrowed colour: the cover's.

### Primary
- **Cover Accent** (`accent`): the default amber stands in until a cover is sampled, then the launcher replaces it with the cover's dominant saturated hue. It fills the one primary action (Play, Write, Update, the suggested memory-card copy), marks the selected item (collection row, save block, sidebar tab) and colours focus rings. Its ink (`accent-ink`) is chosen by contrast at runtime, dark or light.

### Neutral
- **Ground** (`ground`): the window itself, behind the card; what shows at clipped corners.
- **Shell** (`shell`), **Panel** (`panel`), **Sunk** (`sunk`): three tonal steps of warm near-black for surfaces, raised panels and inset footers.
- **Ink** (`ink`) through **Ink 3** (`ink-3`): text, from titles down to hints. `ink-3` is the dimmest text allowed on the panel (4.5:1).
- **Hairlines** (`hairline`, `hairline-mid`, `hairline-soft`) and tonal layers (`lift`, `well`): borders and row backgrounds, as white or black at low alpha so they sit on any tint.

### Semantic
- **Good** (`good`) and **Danger** (`danger`): the save lamps and destructive actions. Never the only carrier of meaning (see Shapes).

### Named Rules
**The Borrowed Colour Rule.** The accent belongs to the cartridge. Never hard-code a brand hue where the accent goes; a greyscale cover resets to the stock amber rather than keeping the last game's.

**The One Fill Rule.** One filled button per screen. Everything else is ghost, text or icon.

## Typography

**Display and body:** Archivo (variable, self-hosted), stretched slightly wide (104–108%) for titles and labels.
**Data:** Spline Sans Mono, tabular numerals, for sizes, paths, times and templates only.

**Character:** a sturdy grotesque that reads as a printed label on a cartridge, with a mono reserved for the numbers a player compares.

### Hierarchy
- **Display** (800, 40px, 1): the game's title on the launcher when there is no logo.
- **Headline** (800, 19px): a dialog's one heading, the memory card's name.
- **Title** (700, 14px): card titles, the collection's name in the corner, list rows.
- **Body** (400, 12.5px, 1.45): explanatory text and hints.
- **Label** (700, 11px, 0.19em, uppercase): section labels and buttons.

### Named Rules
**The Couch Floor Rule.** Nothing that carries meaning is smaller than 11px, and anything read on the launcher from a distance — the collection's name, "Safe to remove", save names — is 12px or more, headlines 14px or more.

**The No Kicker Rule.** No small label above a heading. A collection's name goes where its logo would, in the corner, not stacked over the game's title.

## Layout

The launcher is a fixed 420×630 window (the 3:4 of a cover plus the action row); skins may resize it. Content stacks from the bottom: the art fills, and title, rail and actions sit on a short scrim at the foot. The chrome is a row of icon buttons at the top, shown on hover or focus.

The wizard is a resizable window (1030×660, min 870×520) in three columns: a 148px sidebar, the working column, and a 348px rail with the live launcher preview and the one primary action at its foot. Groups are separated by 14–20px; rows inside a group by 5–8px.

## Elevation & Depth

Depth is tone first: `lift` and `well` layers over the shell, and three neutral steps. Shadows are rare and physical — an offset, soft drop under things that lift (a selected save block, the launcher preview, an icon) — and there is no glow.

### Shadow Vocabulary
- **Lift** (`box-shadow: 0 12px 20px -12px oklch(0 0 0 / 0.85)`): a selected block raised off the grid.
- **Card** (`box-shadow: inset 0 0 0 1px var(--hairline-mid), 0 14px 30px -12px oklch(0 0 0 / 0.8)`): the launcher preview in the wizard.

### Named Rules
**The No Halo Rule.** No zero-offset coloured glow. Selection is a border in the accent and a lift, not light.

## Shapes

Controls are gently squared (5px); build cards and rows a little softer (11px); the launcher's own corners are square (0px), because a transparent rounded corner on a frameless window leaves a wedge for the OS to paint. Pills are for small badges only. Status lamps carry meaning in shape as well as colour: circle for in step, diamond for newer on one side, square for changed on both, ring for nothing yet.

## Components

### Buttons
- **Shape:** gently squared (5px), 46px tall (40–42px in dense panels), uppercase label type.
- **Primary:** the accent fill with its contrast-picked ink. One per screen.
- **Ghost:** a faint `lift` fill with a hairline border, ink text; the danger variant keeps ink until hovered, focused or armed, then turns `danger`.
- **Keys and pad:** each action shows its keycap (↵, E) and, while a pad is in hand, its face-button badge (A, X, Y, B, View) on the corner.

### Icon Buttons
- 34px (28px in the launcher chrome), `well` background, `ink-2` glyph brightening on hover, accent focus ring.

### Build Cards
- `well` background, hairline border, 11px radius, 10×12px padding; a dashed border when empty. States an answer with the way to change it (Choose / Change).

### The Memory Card Stage (signature)
- The memory card as hardware: a slab of smoked plastic with a moulded grip, a recessed label (its capacity, only when the drive reported one, and a stripe of the accent) and the connector's pins along its bottom edge. It hovers over the middle of the stage and leans after the pointer and the turning track; it never moves on its own.
- Saves as game cases on a circular track round it, on a floor seen from slightly above: a ring for five to thirteen, a short arc for fewer, an arc with faded ends for more. The chosen case stands at the front, a step forward and a little larger, turned just off square so its spine shows, with an accent border. Cases round the curve fall into shade rather than shrink to nothing, so the size of the collection reads at a glance.
- A lamp on a case's cover for a save that needs something, in the lamp's shape, readable from across the track.
- Motion is springs towards targets set by the selection alone, drawn only while something moves; the card slots in and the saves fan out of it on opening. Reduced motion puts everything straight in place. Built from CSS 3D transforms (no WebGL), with a flat strip where 3D is missing.
- Under the stage, a fixed-height panel: the save's name, its state with its lamp, only the facts the drive and the PC actually hold, and the one filled button the state calls for.

## Do's and Don'ts

### Do:
- **Do** let the cover be the surface; print chrome over it on a short scrim.
- **Do** take the accent from the cartridge and check its ink for contrast.
- **Do** say every state in words as well as colour, and before a risky action, not after.
- **Do** give every action a key and a pad button, and show them.

### Don't:
- **Don't** put a small label above a heading.
- **Don't** use a zero-offset coloured glow, a radial halo, or glass as decoration.
- **Don't** set meaningful text below 11px, or below 12px on the launcher.
- **Don't** give one pad button a destructive meaning on one screen and a different one on the next.
