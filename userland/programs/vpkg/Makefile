# VeridianOS Package Manager -- vpkg Makefile
#
# Copyright (c) 2025-2026 VeridianOS Contributors
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# Builds the vpkg user-space package management tool.
# Requires the VeridianOS cross-compiler and libc.
#
# Usage:
#   make                    # Build for x86_64 (default)
#   make ARCH=aarch64       # Build for AArch64
#   make ARCH=riscv64       # Build for RISC-V 64
#   make clean              # Remove build artifacts

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
LD      = $(CROSS_PREFIX)gcc

# ========================================================================= #
# Paths                                                                     #
# ========================================================================= #

TOPDIR      := $(dir $(abspath $(lastword $(MAKEFILE_LIST))))
SYSROOT     := $(TOPDIR)../../../toolchain/sysroot
LIBC_DIR    := $(TOPDIR)../../libc

INCDIR      := $(LIBC_DIR)/include
SYS_INCDIR  := $(SYSROOT)/include
LIBC_LIB    := $(LIBC_DIR)/build/$(ARCH)/libc.a
BUILDDIR    := $(TOPDIR)build/$(ARCH)

# ========================================================================= #
# Compiler flags                                                            #
# ========================================================================= #

CFLAGS  = -std=c11
CFLAGS += -nostdlib -nostdinc -ffreestanding
CFLAGS += -isystem $(INCDIR)
CFLAGS += -isystem $(SYS_INCDIR)
# Re-add GCC's freestanding headers
GCC_INCDIR := $(shell $(CC) -print-file-name=include 2>/dev/null)
ifneq ($(GCC_INCDIR),include)
CFLAGS += -isystem $(GCC_INCDIR)
endif
CFLAGS += -Wall -Wextra -Wpedantic
CFLAGS += -Wno-unused-parameter
CFLAGS += -fno-stack-protector
CFLAGS += -fno-builtin
CFLAGS += -O2 -g

# Architecture-specific flags
ifeq ($(ARCH),x86_64)
  CFLAGS += -mno-red-zone -mcmodel=small
else ifeq ($(ARCH),aarch64)
  CFLAGS += -mgeneral-regs-only
else ifeq ($(ARCH),riscv64)
  CFLAGS += -march=rv64gc -mabi=lp64d
endif

# ========================================================================= #
# Linker flags                                                              #
# ========================================================================= #

LDFLAGS  = -nostdlib -static
LDFLAGS += -L$(dir $(LIBC_LIB))

LIBS = -lc -lgcc

# ========================================================================= #
# Sources and objects                                                       #
# ========================================================================= #

SRCS := main.c database.c install.c remove.c query.c
OBJS := $(patsubst %.c,$(BUILDDIR)/%.o,$(SRCS))

TARGET := $(BUILDDIR)/vpkg

# ========================================================================= #
# Targets                                                                   #
# ========================================================================= #

.PHONY: all clean

all: $(TARGET)

$(TARGET): $(OBJS) $(LIBC_LIB) | $(BUILDDIR)
	$(LD) $(LDFLAGS) -o $@ $(OBJS) $(LIBS)
	@echo "Built $(TARGET)"

$(BUILDDIR)/%.o: $(TOPDIR)%.c | $(BUILDDIR)
	$(CC) $(CFLAGS) -c -o $@ $<

$(BUILDDIR):
	mkdir -p $(BUILDDIR)

clean:
	rm -rf $(TOPDIR)build
