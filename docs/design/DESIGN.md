# Zephon: design plan

The mockup is `mockup.html`. The mockup is a design file only. GPUI renders the real window, and the constraints below come from the gpui-kit and GPUI sources.

## Subject, audience, job

- **Subject:** a sender that plays this PC's sound in other rooms. A room is an AirPlay receiver: for example the HomePod minis "Kitchen" and "Upstairs Office", a Samsung "Den Soundbar", a "Basement TV" and an Apple TV "Den".
- **Audience:** a technical user who may already run a desk audio chain (a USB mic and mixer, powered speakers). They think in terms of sources, outputs, levels and latency.
- **Job:** choose where the sound comes from, which rooms play it and how loud each one is, then press Play. The window stays open. Most of the time they only glance at it to answer "is it playing, where, and is anything wrong?"

## Words the UI uses

The UI uses the user's words, not the internal ones.

| Internal | In the UI |
|---|---|
| capture endpoint | **Sound from** |
| receiver | **room** (the list is headed **Rooms**) |
| include checkbox | switch on the room row |
| start / stop | **Play** / **Stop** |
| `streaming (N packets)` | **Playing** (the packet count goes to the log) |
| `connecting...` / `restarting with stable...` | **Connecting…** / **Switching to stable…** |
| latency profile `low (~125 ms)` | **Low 125 ms** in the select. The menu adds the line "lag behind this PC". |
| `loss: N resends/min`, `M lost/min` | **N resends/min**, plus **M lost/min** in amber |
| `Glitchy? Try stable (~257 ms)` | **Glitchy?** Stable adds 92 ms of lag and rides out Wi-Fi drops. **[Switch to stable]** |
| `(offline)` | **Offline** |
| model `AudioAccessory5,1`, `AppleTV14,1` | **HomePod mini**, **Apple TV 4K**. Unknown models such as `TV-55` are shown as they are. |
| `error: ...` | **Didn't connect.** followed by the cause and the fix, for example "No answer within 10 s. The firewall rule is likely missing." |
| reconnect attempt 1 of 3 | **Reconnecting, attempt 1 of 3…** in amber; the footer reads **Reconnecting…** |
| reconnect gave up | **Stopped playing.** followed by the first cause, "Reconnecting failed 3 times." and what to check |

## Concept

The window is a pane of Windows glass with the rooms listed on it, and a **lag line** runs along the bottom. The lag line shows each playing room as a point on a short timeline that starts at "This PC". The user sees at a glance where the sound is going and how far behind the PC each room plays. Everything else stays native to Windows 11, so the lag line is the one thing that stands out.

## Color

There are two token sets, Light and Dark, and the app's follow-system wiring chooses between them. Every surface is a translucent tint over the OS material, so no surface is opaque.

### Base colors

| Name | Light | Dark | Use |
|---|---|---|---|
| Ink | `#1B1E25` | `#F1F3F6` | primary text, room names, the "This PC" origin |
| Ink 2 | `#565C69` | `#A9AFBB` | labels, meta, loss figures, captions |
| Ink 3 | `#868C99` | `#6F7682` | IP addresses, offline names, disabled text (never body text) |
| Lagoon (accent) | `#00708A` | `#5CC8D6` | Play, switch on, slider fill, "Playing", lag markers |
| Amber | `#8F5300` | `#F0B455` | lost/min, the glitch hint, firewall warning |
| Signal red | `#B4232C` | `#FF9494` | the "Didn't connect" error |

Text on the Lagoon accent is `#FFFFFF` in Light and `#062A31` in Dark.

### Glass layers

All values are RGBA. The root tint must keep text readable over any wallpaper.

| Layer | Light | Dark | Role |
|---|---|---|---|
| Root tint (`background`) | `rgba(238,241,246,0.35)` | `rgba(27,30,37,0.40)` | over Mica, which is calm, so the wallpaper color shows. Over Acrylic (the default, which shows busy windows behind), 0.86 / 0.88. |
| Layer 1 (`muted`) | `rgba(255,255,255,0.36)` | `rgba(255,255,255,0.040)` | the rooms group, the footer, rooms that are off |
| Layer 2 (`list_active`) | `rgba(255,255,255,0.66)` | `rgba(255,255,255,0.075)` | rooms that are on |
| Control (`input` fill) | `rgba(255,255,255,0.72)` | `rgba(255,255,255,0.065)` | select, neutral button |
| Stroke (`border`) | `rgba(22,28,45,0.085)` | `rgba(255,255,255,0.075)` | group edge, row dividers |
| Stroke strong | `rgba(22,28,45,0.18)` | `rgba(255,255,255,0.15)` | bottom edge of controls, lag rail |
| Track | `rgba(22,28,45,0.24)` | `rgba(255,255,255,0.30)` | slider rail, outline of a switch that is off |
| Amber wash | `rgba(143,83,0,0.085)` | `rgba(240,180,85,0.10)` | hint and banner fill |
| Red wash | `rgba(180,35,44,0.075)` | `rgba(255,148,148,0.09)` | error fill |

Contrast was checked over the mockup's backdrops. Lagoon on Layer 2 is about 5:1 in Light and about 9:1 in Dark. Ink 2 on the root tint is above 4.5:1 in both themes. Ink 3 is used only for secondary data whose meaning is repeated elsewhere: the IP address, or "Offline", which the dimmed name also shows.

### Mapping to gpui-kit theme tokens

| gpui-kit token | Value |
|---|---|
| `background` | Root tint |
| `foreground` | Ink |
| `muted` | Layer 1 |
| `muted_foreground` | Ink 2 |
| `border` | Stroke |
| `window_border` | Light `rgba(0,0,0,0.14)`, Dark `rgba(255,255,255,0.10)` |
| `title_bar` | transparent (the title bar is part of the glass) |
| `accent` (hover fill in gpui-kit, not the brand color) | Layer 2 |
| `primary` / `primary_hover` / `primary_active` | Lagoon / Light `#00627A`, Dark `#79D4E0` / Light `#005466`, Dark `#4AB4C2` |
| `primary_foreground` | Light `#FFFFFF`, Dark `#062A31` |
| `list` / `list_hover` / `list_active` | Layer 1 / Layer 1 + 4 % ink / Layer 2 |
| `slider_bar` | Lagoon |
| `slider_thumb` | Light `#FFFFFF`, Dark `#454B56` (the Lagoon inner dot is drawn on it) |
| `switch` (off track) | transparent, with a Track-colored outline |
| `switch_thumb` | Ink 2 when off, `primary_foreground` when on |
| `input` | Stroke strong |
| `ring` (focus) | Ink, 2 px, offset 1 px (Windows style) |
| `overlay` | Light `rgba(0,0,0,0.20)`, Dark `rgba(0,0,0,0.45)` |
| warning / danger (if the kit has them; otherwise local constants) | Amber / Signal red, with their washes |

## Type

**Segoe UI Variable** is the only typeface, and it is not embedded. I chose it on purpose, for three reasons:

- The glass, the caption buttons and the system flyouts that open from this window all use Segoe UI Variable. Another face would look wrong next to them.
- Its digits are tabular by default, so 50/72/30, 165/257 and the resends/min figures stay aligned without a monospace face.
- It has an optical-size axis: Small for 12 px, Text for 14 px and Display for the room names and lag figures.

**Fixel** (MacPaw, SIL OFL 1.1, variable weight 100-900 and width 75-100, with `tnum`) was the OFL candidate. Its width axis looked useful for long room names in a narrow window. It does not earn its place, for these reasons:

- GPUI exposes weight but not a width axis, so the main reason to use Fixel would be lost.
- At 12-15 px, its difference from Segoe is too small to notice (see the specimen in the mockup).
- Embedding it adds about 470 KB and a font-loading path for no visible benefit.

If an embedded face is ever wanted, Fixel is the one to use. Ship its static Text instances through `add_fonts`.

| Role | Size / line | Weight | Optical size |
|---|---|---|---|
| Lag figure (`165`) | 20 / 24 | 600 | Display |
| Room name | 15 / 22 | 600 | Display |
| Body, controls, buttons, "Rooms" | 14 / 20 | 400; 600 for "Rooms" and Play | Text |
| Meta, status, loss, captions, title bar | 12 / 16 | 400; 600 for status words | Small |

The scale is 12 / 14 / 15 / 20. The step from 15 to 20 is used only by the lag figures, so they stand out. All text is in sentence case, with no all-caps labels.

## Layout

The window is 560 px wide by default. It works down to 360 px. Everything is left-aligned, and numbers are right-aligned in their own fixed slots.

### Alternatives considered

```
A  Table (the alpha)                 B  Mixer strips                    C  Room list + lag line (chosen)
┌─────────────────────────────┐     ┌─────────────────────────────┐    ┌─────────────────────────────┐
│[x] Kitchen  [prof v] ==o 50 │     │ Kit │ UO  │ Snd │ TV  │ Den │    │ Sound from [Speakers     v] │
│[x] Upstairs [prof v] ===o 72│     │  ║  │  ║  │  ║  │  ║  │     │    │ Rooms                     ↻ │
│[ ] Soundbar [prof v] =o  30 │     │  o  │  ║  │     │  o  │     │    │ ┌ Kitchen  HomePod   Playing ●┐│
│ ...columns fight at 400 px  │     │  ║  │  o  │     │  ║  │     │    │ │ ◁) ━━━━○──── 50 [Balanced v]││
└─────────────────────────────┘     │prof │prof │     │prof │     │    │ │ 2 resends/min               ││
 Every control shows for every       └─────────────────────────────┘    │ ├ Soundbar  Samsung        ○ ┤│
 device, even rooms that are off.    Suits a mixer user, but 5 strips  │ └─────────────────────────────┘│
                                     do not fit in 360 px, and names    │ ├─165───257── lag line        │
                                     truncate to 3 letters.             │ Playing in 3 rooms     [Stop] │
                                                                         └─────────────────────────────┘
```

I chose C. Only rooms that are on show volume, profile and loss, because those settings do nothing for a room that is off. Rooms that are off collapse to a single line. The rows stay in a fixed order (the order they were discovered in) and expand in place, so turning a room on never moves the row under the cursor.

### Main window, playing (560 px)

```
┌──────────────────────────────────────────────────────────┐
│ ◁)) Zephon                                ⚙   ─   □   ✕  │  title bar on the glass; ⚙ = settings
│ Sound from                                Stop to change │
│ [ Speakers (USB Audio DAC)                             ] │  locked while playing (no chevron)
│                                                          │
│ Rooms                                                  ↻ │
│ ╭──────────────────────────────────────────────────────╮ │  one group, Layer 1
│ │ Kitchen  HomePod mini 192.0.2.101      Playing  (●═) │ │  room on: Layer 2
│ │ ◁) ━━━━━━━━━━○─────────────  50   [Balanced 165 ms v]│ │
│ │ 2 resends/min                                        │ │
│ ├──────────────────────────────────────────────────────┤ │
│ │ Upstairs Office  HomePod mini …        Playing  (●═) │ │
│ │ ◁) ━━━━━━━━━━━━━━━━━○──────  72   [Stable 257 ms   v]│ │
│ │ 6 resends/min                                        │ │
│ ├──────────────────────────────────────────────────────┤ │
│ │ Den Soundbar  Samsung HW-S60        …           (═○) │ │  room off: one line
│ ├──────────────────────────────────────────────────────┤ │
│ │ Basement TV   TV-55 192.0.2.14         Playing  (●═) │ │
│ │ ◁) ━━━━━━○─────────────────  30   [Balanced 165 ms v]│ │
│ │ 41 resends/min  3 lost/min                           │ │  amber
│ │ ┌ Glitchy? Stable adds 92 ms …   [Switch to stable] ┐│ │  amber wash
│ ├──────────────────────────────────────────────────────┤ │
│ │ Den Apple TV 4K 192.0.2.7               Offline (═○) │ │  dimmed name
│ ╰──────────────────────────────────────────────────────╯ │
├──────────────────────────────────────────────────────────┤  footer, Layer 1
│                          165ms              257ms        │
│ ┃━━━━━━━━━━━━┼━━━━━━━━━━━━━●━━━━━━━━━━━━━━━━━●━━━━━━━━   │  lag line, 0 to 300 ms
│ This PC                   Kitchen          Upstairs Office│
│                           Basement TV                    │
│ Each room plays this far behind the PC. Rooms aren't     │
│ synced to each other.                                    │
│ Playing in 3 rooms                             [■ Stop]  │
└──────────────────────────────────────────────────────────┘
```

### Narrow (360-440 px)

- The meta text wraps under the room name.
- The volume slider takes the full row, and the profile select moves to its own line below it.
- The lag line keeps its 0-300 ms scale. Name labels sit left-aligned under their marker. The rightmost room's name is right-aligned to the end of the track, so labels do not collide. If two labels would still overlap, the later one moves down one line.

### Stopped

- **Sound from** is an active select again, with a chevron.
- Rooms that are on keep their slider (start volume) and profile, but show no status.
- The lag line shows hollow markers and the caption "Each room will play…".
- The footer reads "2 rooms on", with the accent button **[▶ Play]** on the right.

### Firewall banner (rare)

The banner has two sizes and uses the same button label, **Install firewall rule**, in both.

- **Compact**, one line between Sound from and Rooms: "Firewall rule missing" and **[Install firewall rule]**. It appears as soon as the startup check finds no rule.
- **Full**, which replaces the compact one after a room has failed with the 10 s timeout. It adds a title, the cause ("Rooms send timing replies back to this PC…") and the note "Windows asks for admin permission once."
- After the install and a successful re-check, the banner disappears with no "Installed" message.

## Components (six)

The window uses only six components: the room row, slider, switch, select, button (accent, neutral and small) and banner. The lag line is not a seventh component. It is built from plain divs: a 1 px rail, a 3 px run with a linear gradient, absolutely positioned 15 px dots, and text.

The corner radii have a hierarchy, so not everything gets the same radius:

- window: 8 (OS)
- rooms group: 8
- banner: 6
- controls, hint and error: 4
- switch and dots: fully round

The only shadows are the window's (from the OS) and a 1 px shadow under the slider thumb. The rows sit in one shared group and are separated by hairlines, so there are no separate cards with their own shadows.

## Principles

1. **Native first, one exception.** Windows 11 metrics (32 px controls, 40×20 switch, 20 px slider thumb with an inner dot, 46 px caption buttons) and Segoe UI Variable throughout. The lag line is the only thing that is not stock Windows.
2. **Glass is the backdrop, not decoration.** The window is the only surface that is blurred. Panels are flat translucent tints over it, and no element uses a blur of its own.
3. **Show controls only where they do something.** A room that is off has no slider or profile select. While playing, Sound from shows as plain text with "Stop to change" instead of looking like a broken select.
4. **Numbers in fixed slots.** Volume, lag and loss each have a fixed position with tabular digits, so a glance compares rooms without reading labels.
5. **Hints are offers.** The glitch hint names its cost (+92 ms) and offers one button. The app never changes the profile by itself.
6. **Errors say what to do.** For example: "Didn't connect. No answer within 10 s. The firewall rule is likely missing."

## The memorable element: the lag line

The alpha explained the timing in a 3-line paragraph: rooms are not clock-synced, each profile plays a fixed lag behind the PC, and rooms on different profiles play offset from each other. The lag line shows the same thing as a picture:

- A rail starts at "This PC" (a short ink bar at 0).
- Each playing room is a Lagoon dot at its profile's lag, with the lag figure above it in 20 px Display type and the room names below.
- Rooms on the same profile share one dot, so the offset between Kitchen and Upstairs Office is the visible gap between their dots.
- A faint tick marks the unused Low slot at 125 ms.
- A room that is losing packets keeps its dot, but its name turns amber.

When a profile changes, the dot slides to its new position with a 200 ms ease-out. That is the only animation in the app besides the switch, and it is skipped when Windows' reduce-motion setting is on. The caption under the line says the rest in one sentence.

## Follow system

- The app follows the Windows light/dark setting by default.
- Theme (System, Light, Dark) is the first row of the settings window. Both windows change together, and System follows Windows live.
- The choice is saved in `settings.json`.

The main window shows no theme control. The user sets this once, so it should not take up space next to the rooms. The app keeps only two token sets, and System just means "pick Light or Dark from the OS".

## Settings window

The gear in the main title bar opens a second, smaller window (520 × 640, resizable, scrolls). There is only one: the gear brings it to the front when it is already open. It is the same pane of glass, with the same material, theme, accent and font as the main window, and a change made in it applies to both windows at once. There is no Save button: every change is saved as it is made.

```
┌────────────────────────────────────────────────┐
│ ◁)) Settings                        ─   □   ✕  │  same title bar, named "Settings"
│ Appearance                                     │  section header: 14 semibold, like "Rooms"
│ ╭────────────────────────────────────────────╮ │  one group per section: Layer 1, hairlines
│ │ Theme                        [System    v] │ │  label: 14, Ink
│ │ System follows the Windows setting.        │ │  one-line description: 12, Ink 2
│ ├────────────────────────────────────────────┤ │
│ │ Window material              [Mica      v] │ │
│ │ Mica isn't available here; using Acrylic.  │ │  a fallback or a problem turns the line amber
│ ├────────────────────────────────────────────┤ │
│ │ Font                         [Segoe UI V v] │ │  searchable; each family in its own face
│ ├────────────────────────────────────────────┤ │
│ │ Text size                    [Default   v] │ │
│ ├────────────────────────────────────────────┤ │
│ │ Accent color                 [Lagoon    v] │ │
│ ╰────────────────────────────────────────────╯ │
│ Playback                                       │
│ ╭────────────────────────────────────────────╮ │
│ │ New-room latency             [Balanced  v] │ │
│ │ New-room volume              ━━━━○──── 50  │ │
│ │ Resume on launch                     (═○)  │ │
│ │ Volume follows phone                 (●═)  │ │
│ ╰────────────────────────────────────────────╯ │
│ Rooms                                          │
│ Hidden rooms are left out of the Rooms list…   │  caption: 12, Ink 2
│ ╭────────────────────────────────────────────╮ │
│ │ Kitchen                              (●═)  │ │  every known room, with a Show switch
│ │ HomePod mini                               │ │
│ ╰────────────────────────────────────────────╯ │
│ System                                         │
│ ╭────────────────────────────────────────────╮ │
│ │ Start with Windows                   (═○)  │ │
│ │ Firewall                     [Check again] │ │  [Fix] (accent) when the rule is missing
│ ╰────────────────────────────────────────────╯ │
│ About                                          │
│ ╭────────────────────────────────────────────╮ │
│ │ Zephon           Version 0.1.0-alpha       │ │
│ │ Log                      [Open log folder] │ │
│ │ Settings file         [Open settings file] │ │
│ ╰────────────────────────────────────────────╯ │
└────────────────────────────────────────────────┘
```

- **Layout.** The Windows 11 Settings pattern on our glass: five sections (Appearance, Playback, Rooms, System, About), each one rounded group of rows separated by hairlines, the same component as the rooms list (Layer 1 rows, 8 px group radius, 16 px from the window edge). A row is a label with one line of description in Ink 2 on the left, and its control on the right in a 180 px column (times the text size), so selects line up. Selects are full width in that column; switches and buttons sit at its right edge.
- **Narrow.** Below 440 px (times the text size), the control moves under its label and takes the full row, as the room controls do in the main window.
- **Descriptions** say what the setting does in one line, and turn amber when something is off: the material fell back, the saved font is not installed, the Windows accent could not be read, the firewall rule is missing. The firewall row then offers **Fix** (the accent button) in place of **Check again**.
- **Controls** are the main window's own: the 40 × 20 switch, the slider with the inner dot, and gpui-kit's select and button with the same tokens. No new components.
- **Text size** (Small 0.92×, Default, Large 1.15×) scales the type scale, gpui-kit's rem (so control heights follow), the boxes sized to hold text and the narrow breakpoints. Spacing, the switch and the slider keep their Windows metrics, as in Windows' own text-size setting.
- **Accent.** Lagoon, or the Windows accent. The Windows accent starts from Windows' own variants (Dark 1 in the light theme, Light 2 in the dark theme) and moves in lightness until text on it and the accent on the glass reach 4.5:1, checked against every root tint and the room fills. Text on it is white in Light and a near-black of its hue in Dark, as with Lagoon.

## Review against the brief and the cliché list

**Brief check.**
- *Simple, minimal:* the window has one list, one select and one button, and rooms that are off take a single line.
- *Glassy:* the title bar and every surface are tints over Mica, with no opaque panels.
- *Modern:* Windows 11 control metrics.
- *Dark and light, following system:* two token sets plus the Theme setting.
- *Real states:* the window holds all of them: playing, off, offline, loss, lost, the hint, the error, both firewall banners, stopped, and locked Sound from.

**Revisions made in the plan, before building:**

1. **One card per room with a soft shadow, changed to one grouped list with hairlines.** Separate cards are the SaaS-card cliché. They also hid the difference between rooms that are on and off, which the plan now shows with Layer 2 versus Layer 1.
2. **Monospace for IPs and milliseconds, changed to Segoe's tabular figures.** A small monospace face for data labels is a template tell. Tabular figures give the same alignment.
3. **The explanatory paragraph, changed to the lag line.** The paragraph was the most important information on the screen, and nobody reads it.
4. **The Windows default blue accent, changed to Lagoon (a cool teal).** Default blue would make the app look like an unstyled template. Lagoon is not an acid green, and the dark theme is translucent wallpaper glass, not a near-black slab, which keeps it clear of the "near-black with one bright accent" cliché.
5. **An in-window "Zephon (alpha)" heading and a System/Light/Dark segmented control, both removed.** The title bar already names the app, and the theme is set once, so a single title-bar button is enough.
6. **Separate Refresh buttons for the device list and rooms.** The device list now re-reads itself each time the select opens. Rooms keep one small ↻ icon button for an immediate re-browse.
7. **An embedded OFL face, changed to Segoe UI Variable.** See Type.

The cliché list does not apply anywhere else:
- no cream background or serif face
- no broadsheet rules
- no all-caps eyebrows
- no meta strings joined with middle dots (the IP is set apart by spacing and color)
- no "→" on buttons
- no decorative gradient: the only gradient is the lag run, and it shows the sound travelling out from the PC

**Changes after critiquing the render:**

1. **The lag line was too weak.** In the first render its figures were 12 px grey, its rail was hairline and its dots were 9 px, so it looked like a footnote. It now has 20 px Display figures, 15 px dots with a halo ring, and a gradient run.
2. **The narrow window had a label collision.** "Kitchen" and "Upstairs Office" overlapped there. The rightmost room's label now anchors to the end of the track.
3. **The Light theme read as white cards on glass.** Layer 2 went from 0.84 to 0.66, Layer 1 from 0.46 to 0.36 and the root tint from 0.80 to 0.74, so the wallpaper shows through, and Ink 2 still passes 4.5:1.
4. **The title-bar glyph looked like the Wi-Fi icon.** It is now a small speaker with outgoing arcs.
5. **I removed one decoration.** The device icon inside the Sound from select added nothing.
