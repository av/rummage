# Test fixtures

Made for this repository (no third-party content; CC0). The other fixture files (PNG,
markdown, text, code, WAV) are written by the tests themselves.

- `lighthouse-report.pdf`: two pages with a text layer, written by hand with Python.
- `beach.jpg`: 320x240, drawn with Pillow, with EXIF (make `TestCam`, taken
  `2021:07:14 10:20:30`, GPS 52.225 N 21.01 E).
- `chime.mp3`: 4 s 660 Hz tone, `ffmpeg -f lavfi -i sine=frequency=660:duration=4 -ac 1 -ar 22050 -c:a libmp3lame -b:a 32k`.
- `clip.mp4`: 12 s test pattern with a 440 Hz tone (MPEG-4 part 2 + AAC), `creation_time`
  2022-03-04T05:06:07Z.
