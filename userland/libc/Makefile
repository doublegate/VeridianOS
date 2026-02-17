# VeridianOS libc -- Makefile
#
# Copyright (c) 2025-2026 VeridianOS Contributors
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# Builds a minimal static libc (libc.a) for VeridianOS user-space programs.
# Requires the VeridianOS cross-compiler (x86_64-veridian-gcc or equivalent).
#
# Usage:
#   make                    # Build for x86_64 (default)
#   make ARCH=aarch64       # Build for AArch64
#   make ARCH=riscv64       # Build for RISC-V 64
#   make clean              # Remove build artifacts
#   make install            # Install into sysroot
#
# The Makefile won't work until the cross-compiler is built, but the
# source should be correct for compilation checks.

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
CFLAGS += -isystem $(SYS_INCDIR)
# Re-add GCC's freestanding headers (stdint.h, stddef.h, float.h, limits.h)
# which -nostdinc strips.
GCC_INCDIR := $(shell $(CC) -print-file-name=include 2>/dev/null)
ifneq ($(GCC_INCDIR),include)
CFLAGS += -isystem $(GCC_INCDIR)
endif
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
  CFLAGS += -mgeneral-regs-only
else ifeq ($(ARCH),riscv64)
  CFLAGS += -march=rv64gc -mabi=lp64d
endif

# ========================================================================= #
# Assembler flags                                                           #
# ========================================================================= #

ASFLAGS  = -nostdlib -nostdinc -ffreestanding
ASFLAGS += -isystem $(INCDIR)
ASFLAGS += -isystem $(SYS_INCDIR)
ifneq ($(GCC_INCDIR),include)
ASFLAGS += -isystem $(GCC_INCDIR)
endif
ASFLAGS += -g

# Architecture-specific assembler flags
ifeq ($(ARCH),x86_64)
  ASFLAGS += -mno-red-zone -mcmodel=small
else ifeq ($(ARCH),aarch64)
  ASFLAGS += -mgeneral-regs-only
else ifeq ($(ARCH),riscv64)
  ASFLAGS += -march=rv64gc -mabi=lp64d
endif

# ========================================================================= #
# Sources and objects                                                       #
# ========================================================================= #

# C sources (all architectures)
C_SRCS := $(wildcard $(SRCDIR)/*.c)
C_OBJS := $(patsubst $(SRCDIR)/%.c,$(BUILDDIR)/%.o,$(C_SRCS))

# Assembly sources -- architecture-specific (*_<arch>.S)
ASM_SRCS := $(wildcard $(SRCDIR)/*_$(ARCH).S)
ASM_OBJS := $(patsubst $(SRCDIR)/%.S,$(BUILDDIR)/%.o,$(ASM_SRCS))

OBJS := $(C_OBJS) $(ASM_OBJS)

TARGET := $(BUILDDIR)/libc.a

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

$(BUILDDIR)/%.o: $(SRCDIR)/%.S | $(BUILDDIR)
	$(CC) $(ASFLAGS) -c -o $@ $<

$(BUILDDIR):
	mkdir -p $(BUILDDIR)

clean:
	rm -rf $(TOPDIR)build

# ========================================================================= #
# Install into sysroot                                                      #
# ========================================================================= #

install: $(TARGET)
	mkdir -p $(INSTALL_LIB)
	cp $(TARGET) $(INSTALL_LIB)/libc.a
	@# Copy libc headers into sysroot (alongside veridian/ headers)
	cp -r $(INCDIR)/* $(INSTALL_INC)/
	@echo "Installed libc.a and headers into $(SYSROOT)"
