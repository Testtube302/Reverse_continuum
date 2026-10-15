#!/usr/bin/env python3
"""Check the local adapter against synthetic fixtures and optionally a server cache."""
import argparse
from pathlib import Path
import struct

from continuum_reference_expander import expand


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("image", type=Path)
    parser.add_argument("--server-cache", type=Path)
    args = parser.parse_args()
    image = args.image.read_bytes()
    vectors = Path(__file__).resolve().parents[2] / "docs/research/continuum/expansion-vectors.txt"
    count = 0
    for line in vectors.read_text().splitlines():
        if not line or line.startswith("#"):
            continue
        key, expected = line.split()
        if expand(image, int(key, 16)) != bytes.fromhex(expected):
            raise ValueError(f"synthetic vector {count} does not match")
        count += 1
    print(f"Passed {count} synthetic reference vectors.")
    if args.server_cache:
        cache = args.server_cache.read_bytes()
        if len(cache) != 4 + 2048 * 84 or cache[:4] != bytes.fromhex("28000100"):
            raise ValueError("unsupported server cache")
        for index in range(1024):
            offset = 4 + (1024 + index) * 84
            key = struct.unpack_from("<I", cache, offset)[0]
            if expand(image, key) != cache[offset + 4:offset + 84]:
                raise ValueError(f"independent server cache record {index} does not match")
        print("Passed all 1024 independent server cache records.")


if __name__ == "__main__":
    main()
