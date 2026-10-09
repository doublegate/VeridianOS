"""libsecret 0.21.8.2: no gcry_set_log_handler() with libgcrypt >= 1.12.

libgcrypt 1.12 deprecated gcry_set_log_handler() (gcrypt.h: "This function
has no more effect"), so egg/egg-libgcrypt.c's call to it warns
(-Wdeprecated-declarations). The call and its handler are kept for older
libgcrypt only.

This changes behaviour, despite the header comment: libgcrypt 1.12.4 still
stores and uses the handler (src/misc.c _gcry_set_log_handler, _gcry_logv).
Without it, libgcrypt's own log messages (secure-memory warnings, self-test
errors, log_bug/log_fatal) go to libgpg-error's log stream (stderr, with
its prefix) instead of GLib's "gcrypt" log domain, and a fatal libgcrypt
error ends the process with exit(2) instead of abort(). libgcrypt's
supported route is its own logging; redirecting that (gpgrt_log_set_sink)
would be process-wide, beyond libsecret's business, so it is not done.

Usage: libsecret_gcrypt_log_handler.py <egg/egg-libgcrypt.c>
"""

import sys
from pathlib import Path

path = sys.argv[1]
text = Path(path).read_text()
FIXES = (
    (
        (
            "static void\nlog_handler (gpointer unused, int unknown, const gchar *msg, va_list va)\n"
            "{\n\t/* TODO: Figure out additional arguments */\n"
            '\tg_logv ("gcrypt", G_LOG_LEVEL_MESSAGE, msg, va);\n}\n'
        ),
        (
            "#if GCRYPT_VERSION_NUMBER < 0x010c00 /* deprecated in 1.12 */\n"
            "static void\nlog_handler (gpointer unused, int unknown, const gchar *msg, va_list va)\n"
            "{\n\t/* TODO: Figure out additional arguments */\n"
            '\tg_logv ("gcrypt", G_LOG_LEVEL_MESSAGE, msg, va);\n}\n#endif\n'
        ),
    ),
    (
        "\t\t\tgcry_set_log_handler (log_handler, NULL);\n",
        (
            "#if GCRYPT_VERSION_NUMBER < 0x010c00\n"
            "\t\t\tgcry_set_log_handler (log_handler, NULL);\n#endif\n"
        ),
    ),
)
for old, new in FIXES:
    if new in text:
        continue
    if text.count(old) != 1:
        sys.exit(f"{path}: unexpected source for {old.splitlines()[0]!r}")
    text = text.replace(old, new)
Path(path).write_text(text)
