# How a team works: the animation

A one-minute manim animation of how a fantastic_team team runs.

- **Title:** fantastic_team, unattended work teams.
- **The queue:** you queue tasks, of any kind of work.
- **The workers:** a dispatcher and a mix of AI workers (Haiku, Qwen on a local GPU).
- **Hand-out:** the next task goes to whichever worker is free.
- **The work:** each worker works alone, from exact instructions (read, do, check).
- **Delivery:** the delivery agent holds the keys; workers hold none. Results come back as files, a report, a comment, or a branch / PR.
- **Measured:** every task records time, tokens and cost.
- **You review:** you come back to done work and review it.
- **End:** github.com/aisixteen/fantastic_team.

## Re-render

Source: `how_it_works.py`, scene `HowItWorks`. It needs ffmpeg, cairo and pango:

```
brew install ffmpeg cairo pango
```

Low quality, to check it runs:

```
manim -ql how_it_works.py HowItWorks
```

Final 1080p60 mp4:

```
manim -qh --format mp4 -o how_it_works.mp4 how_it_works.py HowItWorks
```

## Preview for the README

GitHub does not play a video stored in the repo, so the README shows an animated preview instead: `how_it_works-preview.gif`, 960 px wide, 8 frames a second, looping, under 8 MB. It links to `how_it_works.mp4` for the full video.

After each re-render of the mp4 (#126), regenerate the preview:

```
sh media/render-preview.sh
```

It needs ffmpeg (on PATH, or at /opt/homebrew/bin/ffmpeg) and fails if the result is over the 8 MB cap. `sh tests/readme_media.sh` checks that the README links the preview and the mp4, that both files exist, that the preview is animated, 960 px wide and under the cap, and that the script still reproduces it.
