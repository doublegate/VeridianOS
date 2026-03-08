/*
 * VeridianOS libc -- xkbcommon/xkbcommon-keysyms.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * X11 keysym definitions for xkbcommon.
 */

#ifndef _XKBCOMMON_XKBCOMMON_KEYSYMS_H
#define _XKBCOMMON_XKBCOMMON_KEYSYMS_H

/* Special keys */
#define XKB_KEY_NoSymbol                0x000000
#define XKB_KEY_VoidSymbol              0xffffff

/* TTY function keys */
#define XKB_KEY_BackSpace               0xff08
#define XKB_KEY_Tab                     0xff09
#define XKB_KEY_Linefeed                0xff0a
#define XKB_KEY_Clear                   0xff0b
#define XKB_KEY_Return                  0xff0d
#define XKB_KEY_Pause                   0xff13
#define XKB_KEY_Scroll_Lock             0xff14
#define XKB_KEY_Sys_Req                 0xff15
#define XKB_KEY_Escape                  0xff1b
#define XKB_KEY_Delete                  0xffff

/* Cursor control & motion */
#define XKB_KEY_Home                    0xff50
#define XKB_KEY_Left                    0xff51
#define XKB_KEY_Up                      0xff52
#define XKB_KEY_Right                   0xff53
#define XKB_KEY_Down                    0xff54
#define XKB_KEY_Prior                   0xff55
#define XKB_KEY_Page_Up                 0xff55
#define XKB_KEY_Next                    0xff56
#define XKB_KEY_Page_Down               0xff56
#define XKB_KEY_End                     0xff57
#define XKB_KEY_Begin                   0xff58

/* Misc functions */
#define XKB_KEY_Select                  0xff60
#define XKB_KEY_Print                   0xff61
#define XKB_KEY_Execute                 0xff62
#define XKB_KEY_Insert                  0xff63
#define XKB_KEY_Undo                    0xff65
#define XKB_KEY_Redo                    0xff66
#define XKB_KEY_Menu                    0xff67
#define XKB_KEY_Find                    0xff68
#define XKB_KEY_Cancel                  0xff69
#define XKB_KEY_Help                    0xff6a
#define XKB_KEY_Break                   0xff6b
#define XKB_KEY_Mode_switch             0xff7e
#define XKB_KEY_Num_Lock                0xff7f

/* Keypad */
#define XKB_KEY_KP_Space                0xff80
#define XKB_KEY_KP_Tab                  0xff89
#define XKB_KEY_KP_Enter                0xff8d
#define XKB_KEY_KP_F1                   0xff91
#define XKB_KEY_KP_F2                   0xff92
#define XKB_KEY_KP_F3                   0xff93
#define XKB_KEY_KP_F4                   0xff94
#define XKB_KEY_KP_Home                 0xff95
#define XKB_KEY_KP_Left                 0xff96
#define XKB_KEY_KP_Up                   0xff97
#define XKB_KEY_KP_Right                0xff98
#define XKB_KEY_KP_Down                 0xff99
#define XKB_KEY_KP_Prior                0xff9a
#define XKB_KEY_KP_Page_Up              0xff9a
#define XKB_KEY_KP_Next                 0xff9b
#define XKB_KEY_KP_Page_Down            0xff9b
#define XKB_KEY_KP_End                  0xff9c
#define XKB_KEY_KP_Begin                0xff9d
#define XKB_KEY_KP_Insert               0xff9e
#define XKB_KEY_KP_Delete               0xff9f
#define XKB_KEY_KP_Equal                0xffbd
#define XKB_KEY_KP_Multiply             0xffaa
#define XKB_KEY_KP_Add                  0xffab
#define XKB_KEY_KP_Separator            0xffac
#define XKB_KEY_KP_Subtract             0xffad
#define XKB_KEY_KP_Decimal              0xffae
#define XKB_KEY_KP_Divide               0xffaf
#define XKB_KEY_KP_0                    0xffb0
#define XKB_KEY_KP_1                    0xffb1
#define XKB_KEY_KP_2                    0xffb2
#define XKB_KEY_KP_3                    0xffb3
#define XKB_KEY_KP_4                    0xffb4
#define XKB_KEY_KP_5                    0xffb5
#define XKB_KEY_KP_6                    0xffb6
#define XKB_KEY_KP_7                    0xffb7
#define XKB_KEY_KP_8                    0xffb8
#define XKB_KEY_KP_9                    0xffb9

/* Function keys */
#define XKB_KEY_F1                      0xffbe
#define XKB_KEY_F2                      0xffbf
#define XKB_KEY_F3                      0xffc0
#define XKB_KEY_F4                      0xffc1
#define XKB_KEY_F5                      0xffc2
#define XKB_KEY_F6                      0xffc3
#define XKB_KEY_F7                      0xffc4
#define XKB_KEY_F8                      0xffc5
#define XKB_KEY_F9                      0xffc6
#define XKB_KEY_F10                     0xffc7
#define XKB_KEY_F11                     0xffc8
#define XKB_KEY_F12                     0xffc9
#define XKB_KEY_F13                     0xffca
#define XKB_KEY_F14                     0xffcb
#define XKB_KEY_F15                     0xffcc
#define XKB_KEY_F16                     0xffcd
#define XKB_KEY_F17                     0xffce
#define XKB_KEY_F18                     0xffcf
#define XKB_KEY_F19                     0xffd0
#define XKB_KEY_F20                     0xffd1
#define XKB_KEY_F21                     0xffd2
#define XKB_KEY_F22                     0xffd3
#define XKB_KEY_F23                     0xffd4
#define XKB_KEY_F24                     0xffd5

/* Modifier keys */
#define XKB_KEY_Shift_L                 0xffe1
#define XKB_KEY_Shift_R                 0xffe2
#define XKB_KEY_Control_L               0xffe3
#define XKB_KEY_Control_R               0xffe4
#define XKB_KEY_Caps_Lock               0xffe5
#define XKB_KEY_Shift_Lock              0xffe6
#define XKB_KEY_Meta_L                  0xffe7
#define XKB_KEY_Meta_R                  0xffe8
#define XKB_KEY_Alt_L                   0xffe9
#define XKB_KEY_Alt_R                   0xffea
#define XKB_KEY_Super_L                 0xffeb
#define XKB_KEY_Super_R                 0xffec
#define XKB_KEY_Hyper_L                 0xffed
#define XKB_KEY_Hyper_R                 0xffee

/* Latin 1 (ISO 8859-1) */
#define XKB_KEY_space                   0x0020
#define XKB_KEY_exclam                  0x0021
#define XKB_KEY_quotedbl                0x0022
#define XKB_KEY_numbersign              0x0023
#define XKB_KEY_dollar                  0x0024
#define XKB_KEY_percent                 0x0025
#define XKB_KEY_ampersand               0x0026
#define XKB_KEY_apostrophe              0x0027
#define XKB_KEY_parenleft               0x0028
#define XKB_KEY_parenright              0x0029
#define XKB_KEY_asterisk                0x002a
#define XKB_KEY_plus                    0x002b
#define XKB_KEY_comma                   0x002c
#define XKB_KEY_minus                   0x002d
#define XKB_KEY_period                  0x002e
#define XKB_KEY_slash                   0x002f

/* Digits */
#define XKB_KEY_0                       0x0030
#define XKB_KEY_1                       0x0031
#define XKB_KEY_2                       0x0032
#define XKB_KEY_3                       0x0033
#define XKB_KEY_4                       0x0034
#define XKB_KEY_5                       0x0035
#define XKB_KEY_6                       0x0036
#define XKB_KEY_7                       0x0037
#define XKB_KEY_8                       0x0038
#define XKB_KEY_9                       0x0039

#define XKB_KEY_colon                   0x003a
#define XKB_KEY_semicolon               0x003b
#define XKB_KEY_less                    0x003c
#define XKB_KEY_equal                   0x003d
#define XKB_KEY_greater                 0x003e
#define XKB_KEY_question                0x003f
#define XKB_KEY_at                      0x0040

/* Uppercase Latin */
#define XKB_KEY_A                       0x0041
#define XKB_KEY_B                       0x0042
#define XKB_KEY_C                       0x0043
#define XKB_KEY_D                       0x0044
#define XKB_KEY_E                       0x0045
#define XKB_KEY_F                       0x0046
#define XKB_KEY_G                       0x0047
#define XKB_KEY_H                       0x0048
#define XKB_KEY_I                       0x0049
#define XKB_KEY_J                       0x004a
#define XKB_KEY_K                       0x004b
#define XKB_KEY_L                       0x004c
#define XKB_KEY_M                       0x004d
#define XKB_KEY_N                       0x004e
#define XKB_KEY_O                       0x004f
#define XKB_KEY_P                       0x0050
#define XKB_KEY_Q                       0x0051
#define XKB_KEY_R                       0x0052
#define XKB_KEY_S                       0x0053
#define XKB_KEY_T                       0x0054
#define XKB_KEY_U                       0x0055
#define XKB_KEY_V                       0x0056
#define XKB_KEY_W                       0x0057
#define XKB_KEY_X                       0x0058
#define XKB_KEY_Y                       0x0059
#define XKB_KEY_Z                       0x005a

#define XKB_KEY_bracketleft             0x005b
#define XKB_KEY_backslash               0x005c
#define XKB_KEY_bracketright            0x005d
#define XKB_KEY_asciicircum             0x005e
#define XKB_KEY_underscore              0x005f
#define XKB_KEY_grave                   0x0060

/* Lowercase Latin */
#define XKB_KEY_a                       0x0061
#define XKB_KEY_b                       0x0062
#define XKB_KEY_c                       0x0063
#define XKB_KEY_d                       0x0064
#define XKB_KEY_e                       0x0065
#define XKB_KEY_f                       0x0066
#define XKB_KEY_g                       0x0067
#define XKB_KEY_h                       0x0068
#define XKB_KEY_i                       0x0069
#define XKB_KEY_j                       0x006a
#define XKB_KEY_k                       0x006b
#define XKB_KEY_l                       0x006c
#define XKB_KEY_m                       0x006d
#define XKB_KEY_n                       0x006e
#define XKB_KEY_o                       0x006f
#define XKB_KEY_p                       0x0070
#define XKB_KEY_q                       0x0071
#define XKB_KEY_r                       0x0072
#define XKB_KEY_s                       0x0073
#define XKB_KEY_t                       0x0074
#define XKB_KEY_u                       0x0075
#define XKB_KEY_v                       0x0076
#define XKB_KEY_w                       0x0077
#define XKB_KEY_x                       0x0078
#define XKB_KEY_y                       0x0079
#define XKB_KEY_z                       0x007a

#define XKB_KEY_braceleft               0x007b
#define XKB_KEY_bar                     0x007c
#define XKB_KEY_braceright              0x007d
#define XKB_KEY_asciitilde              0x007e

/* XF86 multimedia keys */
#define XKB_KEY_XF86AudioMute           0x1008ff12
#define XKB_KEY_XF86AudioLowerVolume    0x1008ff11
#define XKB_KEY_XF86AudioRaiseVolume    0x1008ff13
#define XKB_KEY_XF86AudioPlay           0x1008ff14
#define XKB_KEY_XF86AudioStop           0x1008ff15
#define XKB_KEY_XF86AudioPrev           0x1008ff16
#define XKB_KEY_XF86AudioNext           0x1008ff17
#define XKB_KEY_XF86MonBrightnessUp     0x1008ff02
#define XKB_KEY_XF86MonBrightnessDown   0x1008ff03
#define XKB_KEY_XF86PowerOff            0x1008ff2a
#define XKB_KEY_XF86Sleep               0x1008ff2f

#endif /* _XKBCOMMON_XKBCOMMON_KEYSYMS_H */
