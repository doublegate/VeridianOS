/*
 * VeridianOS libc -- <wctype.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Wide character classification and mapping in the C locale, the only
 * one this libc has: the ASCII classes, nothing above U+007F belongs to
 * any class, and case mapping changes ASCII letters only. The header
 * declared these without any definition (PR #18 review).
 */

#include <string.h>
#include <wctype.h>

int iswupper(wint_t wc) { return wc >= 'A' && wc <= 'Z'; }
int iswlower(wint_t wc) { return wc >= 'a' && wc <= 'z'; }
int iswalpha(wint_t wc) { return iswupper(wc) || iswlower(wc); }
int iswdigit(wint_t wc) { return wc >= '0' && wc <= '9'; }
int iswalnum(wint_t wc) { return iswalpha(wc) || iswdigit(wc); }
int iswxdigit(wint_t wc)
{
	return iswdigit(wc) || (wc >= 'a' && wc <= 'f') || (wc >= 'A' && wc <= 'F');
}
int iswblank(wint_t wc) { return wc == ' ' || wc == '\t'; }
int iswspace(wint_t wc) { return wc == ' ' || (wc >= '\t' && wc <= '\r'); }
int iswcntrl(wint_t wc) { return wc < 0x20 || wc == 0x7f; }
int iswgraph(wint_t wc) { return wc > 0x20 && wc < 0x7f; }
int iswprint(wint_t wc) { return wc >= 0x20 && wc < 0x7f; }
int iswpunct(wint_t wc) { return iswgraph(wc) && !iswalnum(wc); }

wint_t towupper(wint_t wc) { return iswlower(wc) ? wc - 'a' + 'A' : wc; }
wint_t towlower(wint_t wc) { return iswupper(wc) ? wc - 'A' + 'a' : wc; }

/* wctype_t: 1 + the index of the class in this table; 0 is no class. */
static const struct {
	const char *name;
	int (*test)(wint_t);
} classes[] = {
	{"alnum", iswalnum}, {"alpha", iswalpha}, {"blank", iswblank},
	{"cntrl", iswcntrl}, {"digit", iswdigit}, {"graph", iswgraph},
	{"lower", iswlower}, {"print", iswprint}, {"punct", iswpunct},
	{"space", iswspace}, {"upper", iswupper}, {"xdigit", iswxdigit},
};

wctype_t wctype(const char *name)
{
	for (unsigned long i = 0; i < sizeof classes / sizeof classes[0]; i++)
		if (strcmp(name, classes[i].name) == 0)
			return i + 1;
	return 0;
}

int iswctype(wint_t wc, wctype_t desc)
{
	if (desc == 0 || desc > sizeof classes / sizeof classes[0])
		return 0;
	return classes[desc - 1].test(wc);
}

/* wctrans_t is a pointer type here: these two tags name the mappings. */
static const int trans_toupper = 1;
static const int trans_tolower = 2;

wctrans_t wctrans(const char *name)
{
	if (strcmp(name, "toupper") == 0)
		return &trans_toupper;
	if (strcmp(name, "tolower") == 0)
		return &trans_tolower;
	return 0;
}

wint_t towctrans(wint_t wc, wctrans_t desc)
{
	if (desc == &trans_toupper)
		return towupper(wc);
	if (desc == &trans_tolower)
		return towlower(wc);
	return wc;
}
