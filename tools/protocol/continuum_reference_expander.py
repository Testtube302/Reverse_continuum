#!/usr/bin/env python3
"""Local reference adapter, NOT a standalone reimplementation of key expansion.

Requires unicorn==2.1.4 and AICONTINUUM_RUNTIME_CODE pointing to the explicitly
approved, pinned read-only reference image. No proprietary code is bundled.
Outputs exactly 80 binary bytes. The emulator never calls host Windows APIs.
"""
import hashlib
import os
from pathlib import Path
import struct
import sys

from unicorn import Uc, UcError, UC_ARCH_X86, UC_MODE_32, UC_HOOK_CODE
from unicorn.x86_const import (
    UC_X86_REG_EAX, UC_X86_REG_EBP, UC_X86_REG_ECX, UC_X86_REG_EIP,
    UC_X86_REG_ESP, UC_X86_REG_FPCW, UC_X86_REG_FPTAG,
)

IMAGE_SHA256 = "551c950e264c752c5a0f49903cc9663d4f4c7b2b93a02ca7f34345bea77cad7b"
BASE = 0x400000
SIZE = 0xB8000
RETURN = 0x1001000
CATCH_RETURN = RETURN + 16


def expand(image, seed, *, identity=False):
    if len(image) != SIZE or hashlib.sha256(image).hexdigest() != IMAGE_SHA256:
        raise ValueError("reference image identity does not match the verified build")
    if not 0 <= seed <= 0xFFFFFFFF:
        raise ValueError("key must be an unsigned 32-bit integer")
    emu = Uc(UC_ARCH_X86, UC_MODE_32)
    emu.mem_map(BASE, SIZE)
    emu.mem_write(BASE, image)
    # Fresh synthetic object, output, stack, TLS/exception head and CRT storage.
    emu.mem_map(0x1000000, 0x20000)
    emu.mem_map(0, 0x1000)
    emu.mem_map(BASE + SIZE, 0x10000)
    pack = lambda value: struct.pack("<I", value)
    emu.mem_write(0x4B9E50, pack(0x67F))  # CRT floor: round toward -infinity.
    emu.reg_write(UC_X86_REG_FPCW, 0x27F)
    emu.reg_write(UC_X86_REG_FPTAG, 0xFFFF)
    obj, table, stack = 0x1000000, 0x1000100, 0x1010000
    emu.mem_write(obj, pack(table))
    catches = []

    def words(address, count=1):
        return struct.unpack("<" + "I" * count, emu.mem_read(address, count * 4))

    def handle_exception(cpu, address, size, context):
        if address == CATCH_RETURN:
            frame, saved = catches.pop()
            cpu.mem_write(frame - 4, pack(0xFFFFFFFF))
            cpu.reg_write(UC_X86_REG_ESP, saved)
            cpu.reg_write(UC_X86_REG_EIP, cpu.reg_read(UC_X86_REG_EAX))
            return
        # This expansion uses single integer catches. Interpret their existing
        # x86 C++ metadata; reject other exception shapes rather than guessing.
        esp = cpu.reg_read(UC_X86_REG_ESP)
        value = words(words(esp + 4)[0])[0]
        record = words(0)[0]
        _, handler, state = words(record, 3)
        if bytes(cpu.mem_read(handler, 1)) != b"\xb8":
            raise ValueError("unsupported exception handler")
        info = words(words(handler + 1)[0], 7)
        if info[0] != 0x19930520 or info[3] > 32:
            raise ValueError("unsupported exception metadata")
        matching = []
        for index in range(info[3]):
            low, high, catch_state, count, handlers = words(info[4] + index * 20, 5)
            if low <= state <= high:
                matching.append((catch_state, count, handlers))
        if len(matching) != 1 or matching[0][1] != 1:
            raise ValueError("unsupported catch layout")
        catch_state, _, handlers = matching[0]
        _, _, displacement, target = words(handlers, 4)
        frame = record + 12
        saved = words(frame - 16)[0]
        cpu.mem_write((frame + displacement) & 0xFFFFFFFF, pack(value))
        cpu.mem_write(frame - 4, pack(catch_state))
        catches.append((frame, saved))
        cpu.mem_write(saved - 4, pack(CATCH_RETURN))
        cpu.reg_write(UC_X86_REG_ESP, saved - 4)
        cpu.reg_write(UC_X86_REG_EBP, frame)
        cpu.reg_write(UC_X86_REG_EIP, target)

    emu.hook_add(UC_HOOK_CODE, handle_exception, begin=0x46CA73, end=0x46CA73)
    emu.hook_add(UC_HOOK_CODE, handle_exception, begin=CATCH_RETURN, end=CATCH_RETURN)
    # Login identification uses the first stage only. Session key2 expansion
    # includes the second stage; these tables are not interchangeable.
    for entry in ((0x457D60,) if identity else (0x457D60, 0x458BE0)):
        emu.mem_write(stack, pack(RETURN) + pack(seed))
        emu.reg_write(UC_X86_REG_ESP, stack)
        emu.reg_write(UC_X86_REG_ECX, obj)
        emu.emu_start(entry, RETURN, timeout=2_000_000, count=10_000_000)
        if emu.reg_read(UC_X86_REG_EIP) != RETURN or catches:
            raise ValueError("expansion did not finish within the emulation budget")
    return bytes(emu.mem_read(table, 80))


def main():
    try:
        if len(sys.argv) not in (2, 3) or (len(sys.argv) == 3 and sys.argv[2] != "--identity"):
            raise ValueError("usage: continuum_reference_expander.py <decimal-seed> [--identity]")
        base = Path(sys.executable).parent if getattr(sys, "frozen", False) else Path(__file__).parent
        image_path = Path(os.environ.get("AICONTINUUM_RUNTIME_CODE", base / "runtime-code.bin"))
        image = image_path.read_bytes()
        result = expand(image, int(sys.argv[1], 10), identity=len(sys.argv) == 3)
        sys.stdout.buffer.write(result)
    except (KeyError, OSError, ValueError, UcError) as error:
        print(f"Continuum reference expansion failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
