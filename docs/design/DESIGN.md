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
| latency profile `low (~125 ms)` | **Low 125 ms** under a stop of the profile picker. |
| `loss: N resends/min`, `M lost/min` | **N resends/min**, plus **M lost/min** in amber |
| `Glitchy? Try stable (~257 ms)` | **Glitchy?** Stable adds 92 ms of lag and rides out Wi-Fi drops. **[Switch to stable]** |
| `(offline)` | **Offline** |
| model `AudioAccessory5,1`, `AppleTV14,1` | **HomePod mini**, **Apple TV 4K**. Unknown models such as `TV-55` are shown as they are. |
| `error: ...` | **Didn't connect.** followed by the cause and the fix, for example "No answer within 10 s. The firewall rule is likely missing." |
| reconnect attempt 1 of 3 | **Reconnecting, attempt 1 of 3…** in amber; the footer reads **Reconnecting…** |
| reconnect gave up | **Stopped playing.** followed by the first cause, "Reconnecting failed 3 times." and what to check |

## Concept

The window is a pane of Windows glass with the rooms listed on it and Play in a footer at the bottom. Each room that is on shows its volume and how far behind the PC it plays. Everything stays native to Windows 11.

## Color

There are two token sets, Light and Dark, and the app's follow-system wiring chooses between them. Every surface is a translucent tint over the OS material, so no surface is opaque.

### Base colors

| Name | Light | Dark | Use |
|---|---|---|---|
| Ink | `#1B1E25` | `#F1F3F6` | primary text, room names, the "This PC" origin |
| Ink 2 | `#464B55` | `#D0D3DB` | labels, meta, loss figures, captions |
| Ink 3 | `#616774` | `#B3B7BE` | IP addresses, offline names, an unknown volume, menu hints (never body text) |
| Lagoon (accent) | `#00708A` | `#5CC8D6` | Play, switch on, slider fill, "Playing" |
| Amber | `#8C5200` | `#F0B455` | lost/min, the glitch hint, firewall warning |
| Signal red | `#B4232C` | `#FF9999` | the "Didn't connect" error |

Text on the Lagoon accent is `#FFFFFF` in Light and `#062A31` in Dark.

### Glass layers

All values are RGBA. The root tint must keep text readable over any wallpaper.

| Layer | Light | Dark | Role |
|---|---|---|---|
| Root tint (`background`) | `rgba(238,241,246,0.35)` | `rgba(27,30,37,0.40)` | over Mica, which is calm, so the wallpaper color shows. Over Acrylic (the default, which shows busy windows behind), 0.90 / 0.92. |
| Layer 1 (`muted`) | `rgba(255,255,255,0.36)` | `rgba(255,255,255,0.040)` | the rooms group, the footer, rooms that are off |
| Layer 2 (`list_active`) | `rgba(255,255,255,0.66)` | `rgba(255,255,255,0.075)` | rooms that are on |
| Control (`input` fill) | `rgba(255,255,255,0.72)` | `rgba(255,255,255,0.065)` | select, neutral button |
| Stroke (`border`) | `rgba(22,28,45,0.085)` | `rgba(255,255,255,0.075)` | group edge, row dividers |
| Stroke strong | `rgba(22,28,45,0.18)` | `rgba(255,255,255,0.15)` | bottom edge of controls, lag rail |
| Track | `rgba(22,28,45,0.49)` | `rgba(255,255,255,0.47)` | slider rail, outline of a switch that is off, profile stops, scrollbar thumb |
| Amber wash | `rgba(143,83,0,0.085)` | `rgba(240,180,85,0.10)` | hint and banner fill |
| Red wash | `rgba(180,35,44,0.075)` | `rgba(255,148,148,0.09)` | error fill |

### Contrast

Colors meet WCAG 2.2 AA: 4.5:1 for all text (none of it is large enough for 3:1), and 3:1 for the parts that show a control or its state: the outline of a switch that is off, slider rails, the profile stops, the Lagoon dot on the slider thumb, icons and the focus ring. Disabled controls are exempt, as WCAG allows.

Every surface is a tint over the material, so each color is checked against the worst backdrop the material can put behind it, not a typical one:

- **Acrylic** shows whatever is behind the window, from a black terminal to a white page. Only the root tint bounds it, which is why it is 0.90 / 0.92.
- **Mica** is DWM's own tint (`#F3F3F3` at 50 % / `#202020` at 80 %) over the wallpaper with the wallpaper's luminosity replaced by the tint's, so it stays as bright as greys `#F2`-`#F6` / `#1F`-`#22`. The checks use `#E6`-`#FF` / `#18`-`#2C` as a margin.
- **Solid** is opaque.

GPUI adds alpha when it stacks tints (see `src/ui/style.rs`), so a wash or a control fill is checked as GPUI paints it. A unit test checks each color on each surface it is drawn on; the accent, Lagoon or the Windows accent, is checked the same way. The tightest cases are about 4.6:1: Ink 3 on a room that is on over a white window behind acrylic (Dark), Ink 3 on a room that is off over a black one (Light), and the washes with their amber and red text. Of the parts, the ring of a profile stop against the room around it is about 3.1:1 (Light, a black window behind acrylic).

Ink 3 is never drawn on the root tint: there Light Ink 3 over a black window behind acrylic is only about 4:1. The three inks step down evenly in Dark (about 1.35:1 from Ink to Ink 2 and from Ink 2 to Ink 3), so the IP stays dimmer than the model and an offline name dimmer than both.

Group edges, row dividers and control borders are decoration, not held to 3:1: a control is told apart by its text, fill and caret. The slider thumb's ring likewise; its Lagoon dot shows the value. The glyph in the title bar is a logo, which WCAG exempts, so the accent is not checked on the root tint.

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
- It has an optical-size axis: Small for 12 px, Text for 14 px and Display for the room names.

**Fixel** (MacPaw, SIL OFL 1.1, variable weight 100-900 and width 75-100, with `tnum`) was the OFL candidate. Its width axis looked useful for long room names in a narrow window. It does not earn its place, for these reasons:

- GPUI exposes weight but not a width axis, so the main reason to use Fixel would be lost.
- At 12-15 px, its difference from Segoe is too small to notice (see the specimen in the mockup).
- Embedding it adds about 470 KB and a font-loading path for no visible benefit.

If an embedded face is ever wanted, Fixel is the one to use. Ship its static Text instances through `add_fonts`.

| Role | Size / line | Weight | Optical size |
|---|---|---|---|
| Room name | 15 / 22 | 600 | Display |
| Body, controls, buttons, "Rooms" | 14 / 20 | 400; 600 for "Rooms" and Play | Text |
| Meta, status, loss, captions, title bar | 12 / 16 | 400; 600 for status words | Small |

The scale is 12 / 14 / 15. All text is in sentence case, with no all-caps labels.

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
│ │ ◁) ━━━━━━━━━━━━━━━━━━━━○────────────────────────  50 │ │
│ │ ◷  ○─────────────────────●─────────────────────○     │ │  profile picker, same width
│ │    Low 125 ms     Balanced 165 ms  Stable 257 ms     │ │
│ │ 2 resends/min                                        │ │
│ ├──────────────────────────────────────────────────────┤ │
│ │ Upstairs Office  HomePod mini …        Playing  (●═) │ │
│ │ ◁) ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━○─────────  72 │ │
│ │ ◷  ○─────────────────────○─────────────────────●     │ │
│ │    Low 125 ms     Balanced 165 ms  Stable 257 ms     │ │
│ │ 6 resends/min                                        │ │
│ ├──────────────────────────────────────────────────────┤ │
│ │ Den Soundbar  Samsung HW-S60        …           (═○) │ │  room off: one line
│ ├──────────────────────────────────────────────────────┤ │
│ │ Basement TV   TV-55 192.0.2.14         Playing  (●═) │ │
│ │ ◁) ━━━━━━━━━━━━○────────────────────────────────  30 │ │
│ │ ◷  ○─────────────────────●─────────────────────○     │ │
│ │    Low 125 ms     Balanced 165 ms  Stable 257 ms     │ │
│ │ 41 resends/min  3 lost/min                           │ │  amber
│ │ ┌ Glitchy? Stable adds 92 ms …   [Switch to stable] ┐│ │  amber wash
│ ├──────────────────────────────────────────────────────┤ │
│ │ Den Apple TV 4K 192.0.2.7               Offline (═○) │ │  dimmed name
│ ╰──────────────────────────────────────────────────────╯ │
├──────────────────────────────────────────────────────────┤  footer, Layer 1
│ Rooms on different profiles play up to 92 ms apart. Put  │  sync note, only when profiles differ
│ them all on Stable to line them up.                      │
│ Playing in 3 rooms                             [■ Stop]  │
└──────────────────────────────────────────────────────────┘
```

### Narrow (360-440 px)

- The meta text wraps under the room name.
- The volume slider and the profile picker keep their own lines and shrink with the window. The picker's labels put the figure under the name ("Balanced" over "165 ms"), so the three fit.

### Stopped

- **Sound from** is an active select again, with a chevron.
- Rooms that are on keep their slider (start volume) and profile, but show no status.
- The sync note counts the rooms that will play on Play.
- The footer reads "2 rooms on", with the accent button **[▶ Play]** on the right.
- With no room on, the footer reads "No rooms on" and **Play** is disabled, with the tooltip "Turn on at least one room to play." Switching off the last room while playing stops, so the window lands in this state.

### Firewall banner (rare)

The banner has two sizes and uses the same button label, **Install firewall rule**, in both.

- **Compact**, one line between Sound from and Rooms: "Firewall rule missing" and **[Install firewall rule]**. It appears as soon as the startup check finds no rule.
- **Full**, which replaces the compact one after a room has failed with the 10 s timeout. It adds a title, the cause ("Rooms send timing replies back to this PC…") and the note "Windows asks for admin permission once."
- After the install and a successful re-check, the banner disappears with no "Installed" message.

## Components (six)

The window uses only six components: the room row, slider, switch, select, button (accent, neutral and small) and banner.

The corner radii have a hierarchy, so not everything gets the same radius:

- window: 8 (OS)
- rooms group: 8
- banner: 6
- controls, hint and error: 4
- switch: fully round

The only shadows are the window's (from the OS) and a 1 px shadow under the slider thumb. The level meter is part of the slider, not a component of its own. The rows sit in one shared group and are separated by hairlines, so there are no separate cards with their own shadows.

## Principles

1. **Native first.** Windows 11 metrics (32 px controls, 40×20 switch, 20 px slider thumb with an inner dot, 46 px caption buttons) and Segoe UI Variable throughout.
2. **Glass is the backdrop, not decoration.** The window is the only surface that is blurred. Panels are flat translucent tints over it, and no element uses a blur of its own.
3. **Show controls only where they do something.** A room that is off has no volume slider or profile picker. While playing, Sound from shows as plain text with "Stop to change" instead of looking like a broken select.
4. **Numbers in fixed slots.** Volume, lag and loss each have a fixed position with tabular digits, so a glance compares rooms without reading labels.
5. **Hints are offers.** The glitch hint names its cost (+92 ms) and offers one button. The app never changes the profile by itself.
6. **Errors say what to do.** For example: "Didn't connect. No answer within 10 s. The firewall rule is likely missing."

## The sync note

Rooms are not clock-synced to each other. Each plays its profile's lag behind the PC, so rooms on different profiles play the difference apart. The profile picker in each row already shows the lag behind the PC. The offset between rooms is the one thing the rows do not show, so the footer shows it in one line, and only when the rooms that play are on different profiles:

- Two rooms: "Kitchen and Bedroom TV play 92 ms apart. Put both on Stable to line them up."
- More rooms: "Rooms on different profiles play up to 132 ms apart. Put them all on Stable to line them up."

It suggests the slowest profile in use, because that change makes no room more likely to glitch.

The lag line that used to fill the footer showed the same data as a timeline. Its round dots on a rail looked like slider thumbs, so people tried to drag them, and the lag behind the PC was already in each row's profile select (now the picker).

## The profile picker

Each room that is on picks its latency profile on a line under its volume slider:

- A clock icon, then the volume slider's own 4 px Track rail and 20 px thumb with the Lagoon inner dot, then an empty slot as wide as the volume figure. Both rails start and end at the same x, so the two lines read as one block.
- One hollow 12 px stop per profile, evenly spaced: Low at the left end, Balanced in the middle, Stable at the right end. The rail has no fill, because a profile is a choice, not an amount.
- Under the stops, "Low 125 ms", "Balanced 165 ms" and "Stable 257 ms". The first is left-aligned, the middle centred and the last right-aligned, so none runs past the rail. The chosen name is Ink and semibold, the rest Ink 2.
- Drag the thumb, click the rail, or click a label. The thumb snaps to the nearest stop while dragging, and the label under it turns semibold.
- The labels are ghost buttons: Tab reaches them, Enter picks, and a screen reader reads "Latency profile Stable, 257 ms" with its pressed state. They replace the select's keyboard use, since the slider itself takes no keys.
- A profile change restarts the room's session ("Switching to stable…"), so a drag sends it once, on release, and a label on click (press and release on it), not on every move. Until the controller shows the new profile, or for 1.5 s, the thumb stays where it was put.

The stops are not to scale (125 to 165 ms looks as far as 165 to 257 ms). The sync note covers the offset between rooms, so the picker only has to make the choice clear.

## The level meter

While a room plays, its volume rail shows the level of the sound it plays, like the Windows volume mixer. It shows that the room really plays, and when a song goes quiet.

- The volume fill turns into a dark well: Lagoon at 25 % over the color of text on Lagoon, opaque. A Lagoon bar inside it shows the level, at about 4:1 against the well in both themes (any accent keeps at least 3:1; a test checks it). The bar starts at the left end of the rail and never passes the thumb: a room plays no louder than its volume. The thumb's Lagoon dot still shows the volume, so the well carries no information of its own.
- A 2 px Ink line at 80 % holds the loudest recent level for 1 s, then falls.
- The scale is in dB: the left end is −40 dB, the thumb's edge full scale. The bar rises at once and falls the whole rail in 0.45 s.
- The level is real, not an animation. It is the peak of the audio Zephon sends to the room, shown one profile lag after it was captured, when the room plays it. A packet lost on Wi-Fi still shows; a room that is silent shows no bar.
- With reduced motion the bar moves 4 times a second, with no fall and no peak line.
- Rooms that connect, switch profile or reconnect show no meter until they play.

The meter must never slow the window down. The window's content is drawn once and reused; the meters are painted by a separate layer on top, so a meter frame does not draw the rooms again. The meter draws at most 30 frames a second, only while a room plays and the window is not minimized, and only when a bar it can show moves by a device pixel. GPUI cannot reuse drawn content while an accessibility client is attached (the reused part would drop out of the accessibility tree), so then the whole window is drawn for each meter frame. That costs more, but the meter keeps its full rate: on Windows many apps attach this way (password managers, PowerToys, the touch keyboard host) and stay attached, so a meter slowed for them would be slow for most users.

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
- **Accent.** Lagoon, or the Windows accent. The Windows accent starts from Windows' own variants (Dark 1 in the light theme, Light 2 in the dark theme) and moves in lightness until text on it and the accent on the glass reach 4.5:1, checked against the footer and room fills over the darkest and brightest backdrop each window material allows. Text on it is white in Light and a near-black of its hue in Dark, as with Lagoon.

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
3. **The explanatory paragraph, changed to the lag line, and later to the sync note.** The paragraph was the most important information on the screen, and nobody reads it. The lag line that replaced it was later removed (see The sync note).
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
- no decorative gradient

**Changes after critiquing the render:**

1. **The lag line was too weak.** In the first render its figures were 12 px grey, its rail was hairline and its dots were 9 px, so it looked like a footnote. It now has 20 px Display figures, 15 px dots with a halo ring, and a gradient run.
2. **The narrow window had a label collision.** "Kitchen" and "Upstairs Office" overlapped there. The rightmost room's label now anchors to the end of the track.
3. **The Light theme read as white cards on glass.** Layer 2 went from 0.84 to 0.66, Layer 1 from 0.46 to 0.36 and the root tint from 0.80 to 0.74, so the wallpaper shows through, and Ink 2 still passes 4.5:1.
4. **The title-bar glyph looked like the Wi-Fi icon.** It is now a small speaker with outgoing arcs.
5. **I removed one decoration.** The device icon inside the Sound from select added nothing.
