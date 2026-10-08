#!/usr/bin/env python3
"""libsecret 0.21.8.2: no gcry_set_log_handler() with libgcrypt >= 1.12.

libgcrypt 1.12 made gcry_set_log_handler() a deprecated function that "has
no more effect" (gcrypt.h), so egg/egg-libgcrypt.c's call to it does
nothing and warns (-Wdeprecated-declarations). The call and its handler
are kept for older libgcrypt only; with 1.12 behaviour is unchanged.

Usage: libsecret_gcrypt_log_handler.py <egg/egg-libgcrypt.c>
"""
import sys

path = sys.argv[1]
text = open(path).read()
FIXES = (
    ("static void\nlog_handler (gpointer unused, int unknown, const gchar *msg, va_list va)\n"
     "{\n\t/* TODO: Figure out additional arguments */\n"
     "\tg_logv (\"gcrypt\", G_LOG_LEVEL_MESSAGE, msg, va);\n}\n",
     "#if GCRYPT_VERSION_NUMBER < 0x010c00 /* a no-op since 1.12 */\n"
     "static void\nlog_handler (gpointer unused, int unknown, const gchar *msg, va_list va)\n"
     "{\n\t/* TODO: Figure out additional arguments */\n"
     "\tg_logv (\"gcrypt\", G_LOG_LEVEL_MESSAGE, msg, va);\n}\n#endif\n"),
    ("\t\t\tgcry_set_log_handler (log_handler, NULL);\n",
     "#if GCRYPT_VERSION_NUMBER < 0x010c00\n"
     "\t\t\tgcry_set_log_handler (log_handler, NULL);\n#endif\n"),
)
for old, new in FIXES:
    if new in text:
        continue
    if text.count(old) != 1:
        sys.exit(f"{path}: unexpected source for {old.splitlines()[0]!r}")
    text = text.replace(old, new)
open(path, "w").write(text)
