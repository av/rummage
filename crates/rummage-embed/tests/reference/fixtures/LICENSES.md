# Fixture sources and licences

| File | Source | Licence |
|---|---|---|
| `bread.jpg` | [Loaf of sourdough bread cooling](https://commons.wikimedia.org/wiki/File:Loaf_of_sourdough_bread_cooling.jpg), Wikimedia Commons, 500 px thumbnail | CC0 1.0 |
| `cat.jpg` | [Cat sleeping on the front porch](https://commons.wikimedia.org/wiki/File:Cat_sleeping_on_the_front_porch.jpg), Wikimedia Commons, 500 px thumbnail | CC0 1.0 |
| `lighthouse.jpg` | [St Mary's Lighthouse at high tide](https://commons.wikimedia.org/wiki/File:St_Mary%27s_Lighthouse_at_high_tide.jpg), Wikimedia Commons, 500 px thumbnail | CC0 1.0 |
| `speech_bread.wav` | Generated: `espeak-ng -v en-us -s 145 "Mix flour, water and salt, then let the sourdough rise overnight before you bake the bread in a hot oven."`, resampled to 16 kHz mono with ffmpeg | made for this repo, CC0 1.0 |
| `speech_lighthouse.wav` | Generated: `espeak-ng -v en-gb -s 145 "The lighthouse beam swept across the stormy sea as waves crashed against the rocks."`, resampled to 16 kHz mono | made for this repo, CC0 1.0 |
| `cat_zoom.mp4` | Generated from `cat.jpg`: a 4 s, 10 fps zoom (`ffmpeg -loop 1 -i cat.jpg -vf "scale=1000:-2,zoompan=z='1+0.004*on':x='iw/2-(iw/zoom/2)':y='ih/2-(ih/zoom/2)':d=1:s=480x328:fps=10,format=yuv420p" -t 4 -c:v libopenh264 -b:v 400k -an`) | CC0 1.0 |
