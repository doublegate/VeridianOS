# libwayland-client.a -- VeridianOS Wayland Client Library
#
# Static library providing the Wayland client API for user-space applications.
# Uses raw syscalls only (no libc dependency).
#
# Build: make              (cross-compile for VeridianOS)
# Clean: make clean
# Host:  make CC=gcc AR=ar (for host testing / CI validation)

CC ?= x86_64-veridian-gcc
AR ?= x86_64-veridian-ar
CFLAGS = -Wall -Wextra -O2 -fPIC -nostdlib -ffreestanding \
         -fno-stack-protector -fno-exceptions

TARGET = libwayland-client.a
OBJS = wayland-client.o

all: $(TARGET)

$(TARGET): $(OBJS)
	$(AR) rcs $@ $^

wayland-client.o: wayland-client.c wayland-client.h
	$(CC) $(CFLAGS) -c -o $@ $<

clean:
	rm -f $(OBJS) $(TARGET)

.PHONY: all clean
