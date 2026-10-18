#!/usr/bin/env python3
"""Generate synthetic identity vectors using the pinned local reference image.

Usage: python generate_continuum_identity_vectors.py /path/to/runtime-code.bin
No captured identity, credentials, machine data or host Windows calls are used.
Output contains only synthetic inputs, derived tables and resulting wire bytes.
"""
import argparse
from pathlib import Path
import struct
import zlib

from unicorn import Uc, UC_ARCH_X86, UC_MODE_32
from unicorn.x86_const import UC_X86_REG_ECX, UC_X86_REG_EIP, UC_X86_REG_ESP
from continuum_reference_expander import BASE, SIZE, RETURN, expand


def original_encode(image, nonce, fields):
    # expand checks the image size/hash before any code runs.
    key = expand(image, nonce, identity=True)
    cpu = Uc(UC_ARCH_X86, UC_MODE_32)
    cpu.mem_map(BASE, SIZE)
    cpu.mem_write(BASE, image)
    cpu.mem_map(0x1000000, 0x20000)
    obj, table, source, dest, checksum, stack = (
        0x1000000, 0x1000100, 0x1000200, 0x1000300, 0x1000400, 0x1010000
    )
    pack = lambda *words: struct.pack("<" + "I" * len(words), *words)

    def call(entry, this, *args):
        cpu.mem_write(stack, pack(RETURN, *args))
        cpu.reg_write(UC_X86_REG_ESP, stack)
        cpu.reg_write(UC_X86_REG_ECX, this)
        cpu.emu_start(entry, RETURN, timeout=2_000_000, count=10_000_000)
        if cpu.reg_read(UC_X86_REG_EIP) != RETURN:
            raise ValueError("reference routine exceeded its execution budget")

    plain = pack(nonce, 0, *fields)
    cpu.mem_write(source, plain)
    cpu.mem_write(checksum, pack(0xFFFFFFFF))
    call(0x401A70, checksum, source, 64)
    running_crc = struct.unpack("<I", cpu.mem_read(checksum, 4))[0]
    if running_crc != (zlib.crc32(plain) ^ 0xFFFFFFFF):
        raise ValueError("reference CRC disagrees with independent zlib")
    cpu.mem_write(source + 4, pack(running_crc ^ nonce))
    cpu.mem_write(obj, pack(table))
    cpu.mem_write(table, key)
    call(0x458BF0, obj, dest, source + 4, 60)
    wire = pack(nonce) + bytes(cpu.mem_read(dest, 60))
    call(0x458D00, obj, source + 4, dest, 60)
    if bytes(cpu.mem_read(source + 8, 56)) != pack(*fields):
        raise ValueError("original inverse transform disagrees")
    return key, wire


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("image", type=Path)
    args = parser.parse_args()
    image = args.image.read_bytes()
    print("# Synthetic inputs only; generated with original CRC/encrypt/decrypt routines.")
    print("# nonce(hex) identity_table(20 LE u32) fields(14 LE u32) wire(64 bytes)")
    for nonce in (0, 1, 0xFF, 0x100, 0x12345678, 0x7FFFFFFF, 0x80000000, 0xFFFFFFFF):
        for fields in ([0] * 14, [0xFFFFFFFF] * 14,
                       [(i * 0x9E3779B9 + 0x01020304) & 0xFFFFFFFF for i in range(14)]):
            key, wire = original_encode(image, nonce, fields)
            print(f"{nonce:08x}\t{key.hex()}\t{struct.pack('<14I', *fields).hex()}\t{wire.hex()}")


if __name__ == "__main__":
    main()
