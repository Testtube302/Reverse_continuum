#!/usr/bin/env python3
"""Generate synthetic packet vectors from the pinned MIT nullspace cipher.
Usage: python3 tools/protocol/generate_continuum_vectors.py /path/to/nullspace
Requires clang++ and the revision documented in the research README.
No network access or game installation required. Output is TSV on stdout.
"""
import pathlib
import subprocess
import sys
import tempfile

root = pathlib.Path(sys.argv[1])
revision = subprocess.check_output(['git', '-C', str(root), 'rev-parse', 'HEAD'], text=True).strip()
if revision != '7ebdbc53a052fa62ff246b5f6a8b3347c395e5a4':
    raise SystemExit('Use the pinned nullspace revision from the research README')
crypt = (root / 'src/null/net/security/Crypt.cpp').read_text()
checksum = (root / 'src/null/net/security/Checksum.cpp').read_text()
# Extract the public block transform and CRC table/functions, not security solving.
transform = crypt[crypt.index('void encrypt('):crypt.index('void ContinuumEncrypt::FinalizeExpansion')]
crc = checksum[checksum.index('static u8 crc8_table'):checksum.index('u8 crc8_repeat')]
header = r'''
#include <cstdint>
#include <cstring>
#include <cstdio>
#include <cstdlib>
using u8=uint8_t; using u32=uint32_t; using s32=int32_t;
// Define the zero-rotation case explicitly: the upstream shift by 32 is C++ UB.
u32 rol(u32 x,u32 n){n&=31;return n ? (x<<n)|(x>>(32-n)):x;}
u32 ror(u32 x,u32 n){n&=31;return n ? (x>>n)|(x<<(32-n)):x;}
'''
main = r'''
void hex(const u8* p,unsigned n){for(unsigned i=0;i<n;i++)printf("%02x",p[i]);}
bool escape(const u8* p){return p[0]==255 || (!p[0]&&(p[1]==1||p[1]==16||p[1]==17));}
void emit(const u32* key,const u8* p,unsigned n){
 alignas(8) u8 src[528]={}, enc[528]={}, dec[528]={};
 src[0]=crc8(p,n);memcpy(src+1,p,n);encrypt(enc,src,n+1,key);
 decrypt(dec,enc,n+1,key);if(memcmp(src,dec,n+1))abort();
 hex(reinterpret_cast<const u8*>(key),80);printf("\t");hex(p,n);printf("\t");
 if(escape(enc))printf("ff");hex(enc,n+1);printf("\n");
}
int main(){
 printf("# final table (20 little-endian u32 words)\tplaintext\tciphertext\n");
 for(unsigned k=0;k<3;k++){
  u32 key[20];for(unsigned i=0;i<20;i++)key[i]=k==0?0:k==1?0xffffffffu:i*0x9e3779b9u;
  for(unsigned n: {1u,2u,3u,4u,5u,6u,7u,8u,9u,10u,11u,12u,13u,14u,15u,16u,17u,18u,31u,32u,33u,127u,128u,129u,518u,519u,520u}){
   u8 p[520];for(unsigned i=0;i<n;i++)p[i]=(i*37+n)&255;emit(key,p,n);
  }
 }
 u32 key[20];for(unsigned i=0;i<20;i++)key[i]=i*0x9e3779b9u;
 unsigned seen=0;
 for(u32 i=0;i<2000000 && seen!=15;i++){
  alignas(8) u8 p[8]={},src[16]={},enc[16]={};memcpy(p,&i,4);
  src[0]=crc8(p,8);memcpy(src+1,p,8);encrypt(enc,src,9,key);
  unsigned bit=enc[0]==255?1:enc[0]!=0?0:enc[1]==1?2:enc[1]==16?4:enc[1]==17?8:0;
  if(bit && !(seen&bit)){emit(key,p,8);seen|=bit;}
 }
 if(seen!=15)return 2;
}
'''
with tempfile.TemporaryDirectory() as tmp:
    source = pathlib.Path(tmp)/'vectors.cpp'
    source.write_text(header+'\n#include <initializer_list>\n'+transform+crc+main)
    binary = pathlib.Path(tmp)/'vectors'
    subprocess.run(['clang++','-std=c++17','-O2','-fno-strict-aliasing',str(source),'-o',str(binary)],check=True)
    subprocess.run([str(binary)],check=True)
