#!/usr/bin/env python3
"""Source fixes for the GCC warnings in libvorbis 1.3.7.

- lib/block.c: `if(b->header)_ogg_free(b->header);b->header=NULL;` puts the
  unconditional assignment on the line of the if, which only looks guarded
  (-Wmisleading-indentation). Each assignment moves to its own line.
- lib/floor1.c, fit_line(): y2b is accumulated and never read
  (-Wunused-but-set-variable). The accumulator goes; nothing used it.
- lib/lpc.c, vorbis_lpc_from_data(): for a negative order m the arrays had a
  negative size and aut[0] was read unwritten (-Wmaybe-uninitialized). Such
  an order now returns 0 before allocating; every valid order is unchanged.
- lib/psy.c, _vp_noisemask(): for n <= 0 the function does nothing, but GCC
  could not see that the work array is written before it is passed on
  (-Wmaybe-uninitialized). It now returns at once for n <= 0.

Valid inputs behave as before. Usage: libvorbis_warnings.py <source dir>
"""
import sys

src = sys.argv[1]

FIXES = {
    "lib/block.c": [
        ("  if(b->header)_ogg_free(b->header);b->header=NULL;\n"
         "  if(b->header1)_ogg_free(b->header1);b->header1=NULL;\n"
         "  if(b->header2)_ogg_free(b->header2);b->header2=NULL;\n",
         "  if(b->header)_ogg_free(b->header);\n  b->header=NULL;\n"
         "  if(b->header1)_ogg_free(b->header1);\n  b->header1=NULL;\n"
         "  if(b->header2)_ogg_free(b->header2);\n  b->header2=NULL;\n"),
    ],
    "lib/floor1.c": [
        ("  double xb=0,yb=0,x2b=0,y2b=0,xyb=0,bn=0;\n",
         "  double xb=0,yb=0,x2b=0,xyb=0,bn=0;\n"),
        ("    y2b+=a[i].y2b + a[i].y2a * weight;\n", ""),
        ("    y2b+= *y0 * *y0;\n", ""),
        ("    y2b+= *y1 * *y1;\n", ""),
    ],
    "lib/lpc.c": [
        ("float vorbis_lpc_from_data(float *data,float *lpci,int n,int m){\n"
         "  double *aut=alloca(sizeof(*aut)*(m+1));\n"
         "  double *lpc=alloca(sizeof(*lpc)*(m));\n"
         "  double error;\n"
         "  double epsilon;\n"
         "  int i,j;\n",
         "float vorbis_lpc_from_data(float *data,float *lpci,int n,int m){\n"
         "  double *aut;\n"
         "  double *lpc;\n"
         "  double error;\n"
         "  double epsilon;\n"
         "  int i,j;\n"
         "\n"
         "  if(m<0)return 0.f; /* no coefficients (and no negative-size alloca) */\n"
         "  aut=alloca(sizeof(*aut)*(m+1));\n"
         "  lpc=alloca(sizeof(*lpc)*(m));\n"),
    ],
    "lib/psy.c": [
        ("  int i,n=p->n;\n"
         "  float *work=alloca(n*sizeof(*work));\n"
         "\n"
         "  bark_noise_hybridmp(n,p->bark,logmdct,logmask,",
         "  int i,n=p->n;\n"
         "  float *work;\n"
         "\n"
         "  if(n<=0)return; /* nothing to mask */\n"
         "  work=alloca(n*sizeof(*work));\n"
         "\n"
         "  bark_noise_hybridmp(n,p->bark,logmdct,logmask,"),
    ],
}

for name, fixes in FIXES.items():
    path = f"{src}/{name}"
    text = open(path).read()
    for old, new in fixes:
        if old not in text and (not new or new in text):
            continue  # already applied
        if text.count(old) != 1:
            sys.exit(f"{path}: unexpected source for {old.strip()!r}")
        text = text.replace(old, new)
    open(path, "w").write(text)
