"""Third-party read-back oracle for crates/io/tests/oracle_pillow.rs.

Opens each file named on the command line with Pillow (https://python-pillow.org, MIT-CMU
licence; run as an external program, never linked) and prints one line per file:

    <path>\t<width> <height> <mode> <sample> <sample> ...

or `<path>\tERROR <message>`. Every mode but CMYK is converted to RGBA (straight alpha); CMYK
samples are printed as stored.
"""

import sys

from PIL import Image

KEEP = {"RGBA", "CMYK"}

for path in sys.argv[1:]:
    try:
        with Image.open(path) as im:
            im.load()
            if im.mode not in KEEP:
                im = im.convert("RGBA")
            samples = " ".join(str(b) for b in im.tobytes())
            print(f"{path}\t{im.width} {im.height} {im.mode} {samples}")
    except Exception as e:  # report every failure to the Rust side
        msg = f"{type(e).__name__}: {e}".replace("\n", " ")
        print(f"{path}\tERROR {msg}")
