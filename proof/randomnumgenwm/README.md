# RandomNumGen v1.2 (Windows Mobile) proof

The supplied `randomnumgenwm_v1.2-ce2babfb50e7.cab` (tectrasystems.org
Random Number Generator) was run through PocketHLE's managed-runtime path.

## Result

- The cabinet's `_setup.xml` names the payload `RandomNumGen v1.2.exe`;
  the launcher materialized the long name and routed the image to the
  managed path (host runtime `mono`) on a 240x320 Xvfb display.
- `pe-info` reports the image as **.NET Compact Framework 2.0** (metadata
  version `2.0.0.0`, x86 AnyCPU).
- Interactive taps verified: three taps on the `Generate` menu appended
  `0-1: 0`, `0-1: 1`, `0-1: 0` to the output list — the `Random` class,
  NumericUpDown bounds parsing and ListBox output all behave.
- Screenshots: `startup.png` (first frame), `generate-once.png` (one
  generated result), `generated-three.png` (three results).

## Detector improvements shipped with this proof

1. `pocket-pe::describe_managed_runtime` classifies a CLR metadata
   version string: `v`-prefixed (desktop style, e.g. `v4.0.30319`)
   reports as Microsoft .NET Framework; a bare `N.0.0.0` string — what
   .NET Compact Framework assemblies carry — reports as .NET Compact
   Framework with the two-component product version. Anything else is
   shown raw. Pinned by `clr_version_strings_name_their_platform`.
2. The kernel loader error for managed images no longer claims PocketHLE
   "executes native ARM/MIPS WinCE images only"; it names the detected
   platform and points at the frontend managed-runtime path.
3. `pocket-cli`'s managed window detector now prefers a window owned by
   the launched process (`xdotool search --pid`) over the largest visible
   window on the display, so taps cannot land in an unrelated window on
   an inherited `DISPLAY`; the largest-window search remains as fallback.
