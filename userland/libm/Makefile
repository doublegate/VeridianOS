# VeridianOS libm -- Makefile
#
# Copyright (c) 2025-2026 VeridianOS Contributors
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# Builds a minimal static libm (libm.a) for VeridianOS user-space programs.
# Requires the VeridianOS cross-compiler (x86_64-veridian-gcc or equivalent).
#
# Usage:
#   make                    # Build for x86_64 (default)
#   make ARCH=aarch64       # Build for AArch64
#   make ARCH=riscv64       # Build for RISC-V 64
#   make clean              # Remove build artifacts
#   make install            # Install into sysroot

# ========================================================================= #
# Architecture configuration                                                #
# ========================================================================= #

ARCH ?= x86_64

ifeq ($(ARCH),x86_64)
  CROSS_PREFIX = x86_64-veridian-
else ifeq ($(ARCH),aarch64)
  CROSS_PREFIX = aarch64-veridian-
else ifeq ($(ARCH),riscv64)
  CROSS_PREFIX = riscv64-veridian-
else
  $(error Unsupported ARCH=$(ARCH). Use x86_64, aarch64, or riscv64)
endif

# ========================================================================= #
# Toolchain                                                                 #
# ========================================================================= #

CC      = $(CROSS_PREFIX)gcc
AR      = $(CROSS_PREFIX)ar
RANLIB  = $(CROSS_PREFIX)ranlib

# ========================================================================= #
# Paths                                                                     #
# ========================================================================= #

TOPDIR      := $(dir $(abspath $(lastword $(MAKEFILE_LIST))))
SYSROOT     := $(TOPDIR)../../toolchain/sysroot

INCDIR      := $(TOPDIR)include
LIBC_INCDIR := $(TOPDIR)../libc/include
SYS_INCDIR  := $(SYSROOT)/include
SRCDIR      := $(TOPDIR)src
BUILDDIR    := $(TOPDIR)build/$(ARCH)

# Installation destinations
INSTALL_LIB := $(SYSROOT)/lib/$(ARCH)
INSTALL_INC := $(SYSROOT)/include

# ========================================================================= #
# Compiler flags                                                            #
# ========================================================================= #

CFLAGS  = -std=c11
CFLAGS += -nostdlib -nostdinc -ffreestanding
CFLAGS += -isystem $(INCDIR)
CFLAGS += -isystem $(LIBC_INCDIR)
CFLAGS += -isystem $(SYS_INCDIR)
CFLAGS += -Wall -Wextra -Wpedantic
CFLAGS += -Wno-unused-parameter
CFLAGS += -fno-stack-protector
CFLAGS += -fno-builtin
CFLAGS += -O2
CFLAGS += -g

# Architecture-specific flags
ifeq ($(ARCH),x86_64)
  CFLAGS += -mno-red-zone -mcmodel=small
else ifeq ($(ARCH),aarch64)
  # Note: libm needs floating-point, so we do NOT use -mgeneral-regs-only
  CFLAGS += -march=armv8-a
else ifeq ($(ARCH),riscv64)
  CFLAGS += -march=rv64gc -mabi=lp64d
endif

# ========================================================================= #
# Sources and objects                                                       #
# ========================================================================= #

SRCS := $(wildcard $(SRCDIR)/*.c)
OBJS := $(patsubst $(SRCDIR)/%.c,$(BUILDDIR)/%.o,$(SRCS))

TARGET := $(BUILDDIR)/libm.a

# ========================================================================= #
# Targets                                                                   #
# ========================================================================= #

.PHONY: all clean install

all: $(TARGET)

$(TARGET): $(OBJS) | $(BUILDDIR)
	$(AR) rcs $@ $^
	$(RANLIB) $@
	@echo "Built $(TARGET)"

$(BUILDDIR)/%.o: $(SRCDIR)/%.c | $(BUILDDIR)
	$(CC) $(CFLAGS) -c -o $@ $<

$(BUILDDIR):
	mkdir -p $(BUILDDIR)

clean:
	rm -rf $(TOPDIR)build

# ========================================================================= #
# Install into sysroot                                                      #
# ========================================================================= #

install: $(TARGET)
	mkdir -p $(INSTALL_LIB)
	cp $(TARGET) $(INSTALL_LIB)/libm.a
	@# Copy libm headers into sysroot
	cp -r $(INCDIR)/* $(INSTALL_INC)/
	@echo "Installed libm.a and headers into $(SYSROOT)"
