# Glossary

**Sender**: the program on the Windows PC that captures audio and streams it over AirPlay. The thing this project builds. TuneBlade is an existing sender.

**Receiver**: an AirPlay device that plays the stream, such as a HomePod mini.

**System audio**: everything the PC plays through the capture endpoint, captured as a whole rather than per application.

**RAOP**: the original AirPlay audio protocol ("AirPlay 1"), advertised as `_raop._tcp`.

**AirPlay 2**: Apple's newer protocol, advertised as `_airplay._tcp`, with its own pairing, encryption, and timing model.

**Transient pairing**: the AirPlay 2 pairing mode in which a sender pairs for one session with a fixed, well-known PIN, without being stored as a trusted controller in the Home app.

**Capture endpoint**: the Windows playback device whose output the sender records as its system audio. It is chosen and remembered by the user, and is not necessarily the Windows default device.

**Lag**: how far the receiver's sound trails the same sound from the PC's local speakers, measured by ear or with a microphone. The main latency figure for this project.

**Latency profile**: a named trade-off between lag and robustness against network glitches (`low`, `balanced`, `stable`), chosen by the user.

**Silence fill**: sending silent audio while nothing plays on the PC, so the receiver keeps the session open.

**Drift**: the slow divergence between the capture endpoint's clock and the stream's clock, which would grow or drain buffered audio if not corrected.

**Stereo pair**: two receivers grouped in the Home app to play the left and right channels of one stream.

**Multi-room**: playing the same stream on several receivers at once, in sync.
