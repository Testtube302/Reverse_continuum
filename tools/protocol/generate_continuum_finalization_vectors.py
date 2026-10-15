#!/usr/bin/env python3
"""Print synthetic table-finalization vectors using the pinned public C++ reference.
Usage: python3 tools/protocol/generate_continuum_finalization_vectors.py /path/to/nullspace
The reference MD5 implementation is derived from the RSA Data Security, Inc.
MD5 Message Digest Algorithm; see docs/research/continuum/LICENSE-md5.
This does not perform key2 expansion or contact a security service.
"""
import pathlib
import subprocess
import sys
import tempfile

root = pathlib.Path(sys.argv[1]).resolve()
revision = subprocess.check_output(['git', '-C', str(root), 'rev-parse', 'HEAD'], text=True).strip()
if revision != '7ebdbc53a052fa62ff246b5f6a8b3347c395e5a4':
    raise SystemExit('Use the pinned nullspace revision from the research README')
security = root / 'src/null/net/security'
crypt = (security / 'Crypt.cpp').read_text()
finalize = crypt[crypt.index('void ContinuumEncrypt::FinalizeExpansion'):crypt.index('size_t ContinuumEncrypt::Encrypt')]
header = r'''
#include <cstdint>
#include <cstdio>
#include "MD5.h"
using u8=uint8_t; using u32=uint32_t;
struct ContinuumEncrypt {
 enum class State { Waiting };
 u32 expanded_key[20]; u32 key1; State state;
 void FinalizeExpansion(u32 key);
};
void table(const u32* p){for(unsigned i=0;i<20;i++)for(unsigned j=0;j<4;j++)printf("%02x",(p[i]>>(j*8))&255);}
'''
main = r'''
int main(){
 printf("# key1 (hex u32)\texpanded table (little endian)\tfinal session table (little endian)\n");
 for(unsigned n=0;n<32;n++){
  u32 seed=n*0x9e3779b9u, key1=n%4==0?0:n%4==1?1:n%4==2?0x8e6ac060u:0xffffffffu;
  ContinuumEncrypt c{};
  for(unsigned i=0;i<20;i++){
   seed=seed*1664525u+1013904223u;
   c.expanded_key[i]=n<4?0:n<8?0xffffffffu:seed;
  }
  printf("%08x\t",key1);table(c.expanded_key);printf("\t");
  c.FinalizeExpansion(key1);table(c.expanded_key);printf("\n");
 }
}
'''
with tempfile.TemporaryDirectory() as tmp:
    source = pathlib.Path(tmp)/'finalization.cpp'
    source.write_text(header+finalize+main)
    binary = pathlib.Path(tmp)/'finalization'
    subprocess.run(['clang++','-std=c++17','-O2','-I',str(security),str(source),str(security/'MD5.cpp'),'-o',str(binary)],check=True)
    subprocess.run([str(binary)],check=True)
