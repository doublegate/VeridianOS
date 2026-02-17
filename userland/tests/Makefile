# VeridianOS End-to-End Test Programs -- Makefile
#
# Copyright (c) 2025-2026 VeridianOS Contributors
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# Builds test programs against the VeridianOS sysroot and libc.
#
# Usage:
#   make                        # Build all tests for x86_64 (default)
#   make ARCH=aarch64           # Build all tests for AArch64
#   make ARCH=riscv64           # Build all tests for RISC-V 64
#   make minimal                # Build only the minimal (no-libc) test
#   make hello                  # Build only the libc test
#   make clean                  # Remove build artifacts
#   make info                   # Show ELF info for built binaries
#
# Prerequisites:
#   - Cross-compiler at TOOLCHAIN_PREFIX (default: /opt/veridian/toolchain)
#   - libc.a installed in sysroot (run: cd ../libc && make install)
#   - crt0.o assembled and installed in sysroot

# ========================================================================= #
# Architecture configuration                                                #
# ========================================================================= #

ARCH ?= x86_64

# Toolchain location (override with environment or command-line)
TOOLCHAIN_PREFIX ?= /opt/veridian/toolchain

# Cross-compiler binary prefix
ifeq ($(ARCH),x86_64)
  CROSS_PREFIX = $(TOOLCHAIN_PREFIX)/bin/x86_64-veridian-
else ifeq ($(ARCH),aarch64)
  CROSS_PREFIX = $(TOOLCHAIN_PREFIX)/bin/aarch64-veridian-
else ifeq ($(ARCH),riscv64)
  CROSS_PREFIX = $(TOOLCHAIN_PREFIX)/bin/riscv64-veridian-
else
  $(error Unsupported ARCH=$(ARCH). Use x86_64, aarch64, or riscv64)
endif

# ========================================================================= #
# Toolchain                                                                 #
# ========================================================================= #

CC       = $(CROSS_PREFIX)gcc
LD       = $(CROSS_PREFIX)ld
OBJCOPY  = $(CROSS_PREFIX)objcopy
READELF  = $(CROSS_PREFIX)readelf
SIZE     = $(CROSS_PREFIX)size

# ========================================================================= #
# Paths                                                                     #
# ========================================================================= #

TOPDIR      := $(dir $(abspath $(lastword $(MAKEFILE_LIST))))
SYSROOT     := $(TOPDIR)../../toolchain/sysroot

# libc headers and libraries
LIBC_INCDIR := $(TOPDIR)../libc/include
SYS_INCDIR  := $(SYSROOT)/include
LIBC_LIBDIR := $(SYSROOT)/lib/$(ARCH)

# CRT files
CRT0        := $(SYSROOT)/lib/$(ARCH)/crt0.o

# Build output directory
BUILDDIR    := $(TOPDIR)build/$(ARCH)

# ========================================================================= #
# Compiler flags                                                            #
# ========================================================================= #

# Common flags for freestanding compilation against the VeridianOS sysroot
CFLAGS_COMMON  = -std=c11
CFLAGS_COMMON += -Wall -Wextra -Wpedantic
CFLAGS_COMMON += -Wno-unused-parameter
CFLAGS_COMMON += -O2 -g
CFLAGS_COMMON += -static

# Flags for libc-linked programs (hello)
CFLAGS_LIBC  = $(CFLAGS_COMMON)
CFLAGS_LIBC += -nostdinc
CFLAGS_LIBC += -isystem $(LIBC_INCDIR)
CFLAGS_LIBC += -isystem $(SYS_INCDIR)
CFLAGS_LIBC += -fno-stack-protector
CFLAGS_LIBC += -ffreestanding

# Flags for minimal (no-libc) programs
CFLAGS_MINIMAL  = -nostdlib -nostdinc -ffreestanding
CFLAGS_MINIMAL += -Wall -Wextra -O2 -g
CFLAGS_MINIMAL += -static

# Linker flags for libc-linked programs
LDFLAGS_LIBC  = -static -nostdlib
LDFLAGS_LIBC += -L$(LIBC_LIBDIR)

# Architecture-specific flags
ifeq ($(ARCH),x86_64)
  CFLAGS_COMMON  += -mno-red-zone -mcmodel=small
  CFLAGS_MINIMAL += -mno-red-zone -mcmodel=small
else ifeq ($(ARCH),aarch64)
  CFLAGS_COMMON  += -mgeneral-regs-only
  CFLAGS_MINIMAL += -mgeneral-regs-only
else ifeq ($(ARCH),riscv64)
  CFLAGS_COMMON  += -march=rv64gc -mabi=lp64d
  CFLAGS_MINIMAL += -march=rv64gc -mabi=lp64d
endif

# ========================================================================= #
# Targets                                                                   #
# ========================================================================= #

.PHONY: all clean info

all: $(BUILDDIR)/hello $(BUILDDIR)/minimal

# --- hello: Full libc test ---
# Links with crt0.o + libc.a.  CRT0 provides _start which calls main().
$(BUILDDIR)/hello: hello.c | $(BUILDDIR)
	$(CC) $(CFLAGS_LIBC) $(LDFLAGS_LIBC) \
		-o $@ \
		$(CRT0) $< \
		-lc
	@echo "Built: $@ ($(ARCH))"

# --- minimal: No-libc test ---
# Provides its own _start, uses raw syscalls.  No CRT, no libc.
$(BUILDDIR)/minimal: minimal.c | $(BUILDDIR)
	$(CC) $(CFLAGS_MINIMAL) -o $@ $<
	@echo "Built: $@ ($(ARCH))"

$(BUILDDIR):
	mkdir -p $(BUILDDIR)

clean:
	rm -rf $(TOPDIR)build

# ========================================================================= #
# Info: show ELF details for built binaries                                 #
# ========================================================================= #

info: all
	@echo "========================================"
	@echo "  minimal ($(ARCH))"
	@echo "========================================"
	@file $(BUILDDIR)/minimal 2>/dev/null || true
	@$(READELF) -h $(BUILDDIR)/minimal 2>/dev/null || readelf -h $(BUILDDIR)/minimal 2>/dev/null || true
	@$(SIZE) $(BUILDDIR)/minimal 2>/dev/null || size $(BUILDDIR)/minimal 2>/dev/null || true
	@echo ""
	@echo "========================================"
	@echo "  hello ($(ARCH))"
	@echo "========================================"
	@file $(BUILDDIR)/hello 2>/dev/null || true
	@$(READELF) -h $(BUILDDIR)/hello 2>/dev/null || readelf -h $(BUILDDIR)/hello 2>/dev/null || true
	@$(SIZE) $(BUILDDIR)/hello 2>/dev/null || size $(BUILDDIR)/hello 2>/dev/null || true
