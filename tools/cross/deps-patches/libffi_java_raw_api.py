"""libffi: the deprecated Java raw API calls its own deprecated functions.

Each deprecated entry point becomes a static helper with the original body,
followed by the public deprecated function as a one-line wrapper (kept in
the same preprocessor conditionals); internal callers use the helpers, so
building libffi no longer warns about its own deprecations. Behaviour and
the exported API are unchanged.

Usage: python3 libffi_java_raw_api.py <libffi>/src/java_raw_api.c
"""
import re, sys
p = sys.argv[1]
s = open(p).read()
if "java_raw_size_impl" in s:
    sys.exit(0)
# (return type, public name, impl name, call-through expression)
funcs = [
    ("size_t", "ffi_java_raw_size", "java_raw_size_impl", "return java_raw_size_impl (cif);"),
    ("void", "ffi_java_raw_to_ptrarray", "java_raw_to_ptrarray_impl", "java_raw_to_ptrarray_impl (cif, raw, args);"),
    ("void", "ffi_java_ptrarray_to_raw", "java_ptrarray_to_raw_impl", "java_ptrarray_to_raw_impl (cif, args, raw);"),
    ("ffi_status", "ffi_prep_java_raw_closure_loc", "java_prep_raw_closure_loc_impl",
     "return java_prep_raw_closure_loc_impl (cl, cif, fun, user_data, codeloc);"),
]
for ret, pub, impl, call in funcs:
    m = re.search(r"\n" + ret + r"\n" + pub + r" (\([^{]*?\))\n\{\n", s, re.S)
    if not m:
        sys.exit("libffi java_raw_api.c: definition of %s not found" % pub)
    params = m.group(1)
    end = s.index("\n}\n", m.end()) + 3
    body = s[m.end():end]
    wrapper = "\n%s\n%s %s\n{\n  %s\n}\n" % (ret, pub, params, call)
    s = s[:m.start()] + "\nstatic " + ret + "\n" + impl + " " + params + "\n{\n" + body + wrapper + s[end:]
# Internal calls go to the helpers, not the deprecated entry points.
for old, new in (
    ("ffi_java_raw_to_ptrarray (cif, raw, avalue);", "java_raw_to_ptrarray_impl (cif, raw, avalue);"),
    ("alloca (ffi_java_raw_size (cif))", "alloca (java_raw_size_impl (cif))"),
    ("ffi_java_ptrarray_to_raw (cif, avalue, raw);", "java_ptrarray_to_raw_impl (cif, avalue, raw);"),
    ("return ffi_prep_java_raw_closure_loc (cl, cif, fun, user_data, cl);",
     "return java_prep_raw_closure_loc_impl (cl, cif, fun, user_data, cl);"),
):
    if s.count(old) != 1:
        sys.exit("libffi java_raw_api.c: call %r not found" % old)
    s = s.replace(old, new)
open(p, "w").write(s)
