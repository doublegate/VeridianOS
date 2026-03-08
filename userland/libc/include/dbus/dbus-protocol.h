/*
 * VeridianOS libc -- <dbus/dbus-protocol.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * D-Bus protocol constants: type codes, well-known names, flags.
 */

#ifndef _DBUS_DBUS_PROTOCOL_H
#define _DBUS_DBUS_PROTOCOL_H

/* ========================================================================= */
/* Type codes                                                                */
/* ========================================================================= */

#define DBUS_TYPE_INVALID       ((int) '\0')
#define DBUS_TYPE_BYTE          ((int) 'y')
#define DBUS_TYPE_BOOLEAN       ((int) 'b')
#define DBUS_TYPE_INT16         ((int) 'n')
#define DBUS_TYPE_UINT16        ((int) 'q')
#define DBUS_TYPE_INT32         ((int) 'i')
#define DBUS_TYPE_UINT32        ((int) 'u')
#define DBUS_TYPE_INT64         ((int) 'x')
#define DBUS_TYPE_UINT64        ((int) 't')
#define DBUS_TYPE_DOUBLE        ((int) 'd')
#define DBUS_TYPE_STRING        ((int) 's')
#define DBUS_TYPE_OBJECT_PATH   ((int) 'o')
#define DBUS_TYPE_SIGNATURE     ((int) 'g')
#define DBUS_TYPE_UNIX_FD       ((int) 'h')

/* Container types */
#define DBUS_TYPE_ARRAY         ((int) 'a')
#define DBUS_TYPE_VARIANT       ((int) 'v')
#define DBUS_STRUCT_BEGIN_CHAR  ((int) '(')
#define DBUS_STRUCT_END_CHAR   ((int) ')')
#define DBUS_DICT_ENTRY_BEGIN_CHAR ((int) '{')
#define DBUS_DICT_ENTRY_END_CHAR   ((int) '}')

#define DBUS_TYPE_STRUCT        ((int) 'r')
#define DBUS_TYPE_DICT_ENTRY    ((int) 'e')

/* Number of basic type codes */
#define DBUS_NUMBER_OF_TYPES    16

/* ========================================================================= */
/* Message types                                                             */
/* ========================================================================= */

#define DBUS_MESSAGE_TYPE_INVALID       0
#define DBUS_MESSAGE_TYPE_METHOD_CALL   1
#define DBUS_MESSAGE_TYPE_METHOD_RETURN 2
#define DBUS_MESSAGE_TYPE_ERROR         3
#define DBUS_MESSAGE_TYPE_SIGNAL        4

#define DBUS_NUM_MESSAGE_TYPES          5

/* ========================================================================= */
/* Header field codes                                                        */
/* ========================================================================= */

#define DBUS_HEADER_FIELD_INVALID        0
#define DBUS_HEADER_FIELD_PATH           1
#define DBUS_HEADER_FIELD_INTERFACE      2
#define DBUS_HEADER_FIELD_MEMBER         3
#define DBUS_HEADER_FIELD_ERROR_NAME     4
#define DBUS_HEADER_FIELD_REPLY_SERIAL   5
#define DBUS_HEADER_FIELD_DESTINATION    6
#define DBUS_HEADER_FIELD_SENDER         7
#define DBUS_HEADER_FIELD_SIGNATURE      8
#define DBUS_HEADER_FIELD_UNIX_FDS       9

#define DBUS_HEADER_FIELD_LAST           DBUS_HEADER_FIELD_UNIX_FDS

/* ========================================================================= */
/* Message flags                                                             */
/* ========================================================================= */

#define DBUS_HEADER_FLAG_NO_REPLY_EXPECTED  0x1
#define DBUS_HEADER_FLAG_NO_AUTO_START      0x2
#define DBUS_HEADER_FLAG_ALLOW_INTERACTIVE_AUTHORIZATION 0x4

/* ========================================================================= */
/* Well-known bus names, paths, and interfaces                               */
/* ========================================================================= */

#define DBUS_SERVICE_DBUS           "org.freedesktop.DBus"
#define DBUS_PATH_DBUS              "/org/freedesktop/DBus"
#define DBUS_PATH_LOCAL             "/org/freedesktop/DBus/Local"

#define DBUS_INTERFACE_DBUS         "org.freedesktop.DBus"
#define DBUS_INTERFACE_MONITORING   "org.freedesktop.DBus.Monitoring"
#define DBUS_INTERFACE_VERBOSE      "org.freedesktop.DBus.Verbose"
#define DBUS_INTERFACE_INTROSPECTABLE "org.freedesktop.DBus.Introspectable"
#define DBUS_INTERFACE_PROPERTIES   "org.freedesktop.DBus.Properties"
#define DBUS_INTERFACE_PEER         "org.freedesktop.DBus.Peer"
#define DBUS_INTERFACE_LOCAL        "org.freedesktop.DBus.Local"

/* ========================================================================= */
/* Name ownership flags and replies                                          */
/* ========================================================================= */

#define DBUS_NAME_FLAG_ALLOW_REPLACEMENT   0x1
#define DBUS_NAME_FLAG_REPLACE_EXISTING    0x2
#define DBUS_NAME_FLAG_DO_NOT_QUEUE        0x4

#define DBUS_REQUEST_NAME_REPLY_PRIMARY_OWNER  1
#define DBUS_REQUEST_NAME_REPLY_IN_QUEUE       2
#define DBUS_REQUEST_NAME_REPLY_EXISTS         3
#define DBUS_REQUEST_NAME_REPLY_ALREADY_OWNER  4

#define DBUS_RELEASE_NAME_REPLY_RELEASED       1
#define DBUS_RELEASE_NAME_REPLY_NON_EXISTENT   2
#define DBUS_RELEASE_NAME_REPLY_NOT_OWNER      3

/* ========================================================================= */
/* Start reply                                                               */
/* ========================================================================= */

#define DBUS_START_REPLY_SUCCESS         1
#define DBUS_START_REPLY_ALREADY_RUNNING 2

/* ========================================================================= */
/* Bus names                                                                 */
/* ========================================================================= */

#define DBUS_MAXIMUM_NAME_LENGTH         255
#define DBUS_MAXIMUM_MATCH_RULE_LENGTH   1024
#define DBUS_MAXIMUM_MESSAGE_LENGTH      (128 * 1024 * 1024)
#define DBUS_MAXIMUM_MESSAGE_UNIX_FDS    1024
#define DBUS_MAXIMUM_TYPE_RECURSION_DEPTH 32
#define DBUS_MAXIMUM_ARRAY_LENGTH        (64 * 1024 * 1024)
#define DBUS_MAXIMUM_SIGNATURE_LENGTH    255

/* ========================================================================= */
/* Well-known errors                                                         */
/* ========================================================================= */

#define DBUS_ERROR_FAILED                   "org.freedesktop.DBus.Error.Failed"
#define DBUS_ERROR_NO_MEMORY                "org.freedesktop.DBus.Error.NoMemory"
#define DBUS_ERROR_SERVICE_UNKNOWN          "org.freedesktop.DBus.Error.ServiceUnknown"
#define DBUS_ERROR_NAME_HAS_NO_OWNER        "org.freedesktop.DBus.Error.NameHasNoOwner"
#define DBUS_ERROR_NO_REPLY                 "org.freedesktop.DBus.Error.NoReply"
#define DBUS_ERROR_IO_ERROR                 "org.freedesktop.DBus.Error.IOError"
#define DBUS_ERROR_BAD_ADDRESS              "org.freedesktop.DBus.Error.BadAddress"
#define DBUS_ERROR_NOT_SUPPORTED            "org.freedesktop.DBus.Error.NotSupported"
#define DBUS_ERROR_LIMITS_EXCEEDED          "org.freedesktop.DBus.Error.LimitsExceeded"
#define DBUS_ERROR_ACCESS_DENIED            "org.freedesktop.DBus.Error.AccessDenied"
#define DBUS_ERROR_AUTH_FAILED              "org.freedesktop.DBus.Error.AuthFailed"
#define DBUS_ERROR_NO_SERVER                "org.freedesktop.DBus.Error.NoServer"
#define DBUS_ERROR_TIMEOUT                  "org.freedesktop.DBus.Error.Timeout"
#define DBUS_ERROR_NO_NETWORK               "org.freedesktop.DBus.Error.NoNetwork"
#define DBUS_ERROR_ADDRESS_IN_USE           "org.freedesktop.DBus.Error.AddressInUse"
#define DBUS_ERROR_DISCONNECTED             "org.freedesktop.DBus.Error.Disconnected"
#define DBUS_ERROR_INVALID_ARGS             "org.freedesktop.DBus.Error.InvalidArgs"
#define DBUS_ERROR_FILE_NOT_FOUND           "org.freedesktop.DBus.Error.FileNotFound"
#define DBUS_ERROR_FILE_EXISTS              "org.freedesktop.DBus.Error.FileExists"
#define DBUS_ERROR_UNKNOWN_METHOD           "org.freedesktop.DBus.Error.UnknownMethod"
#define DBUS_ERROR_UNKNOWN_OBJECT           "org.freedesktop.DBus.Error.UnknownObject"
#define DBUS_ERROR_UNKNOWN_INTERFACE        "org.freedesktop.DBus.Error.UnknownInterface"
#define DBUS_ERROR_UNKNOWN_PROPERTY         "org.freedesktop.DBus.Error.UnknownProperty"
#define DBUS_ERROR_PROPERTY_READ_ONLY       "org.freedesktop.DBus.Error.PropertyReadOnly"
#define DBUS_ERROR_MATCH_RULE_NOT_FOUND     "org.freedesktop.DBus.Error.MatchRuleNotFound"
#define DBUS_ERROR_MATCH_RULE_INVALID       "org.freedesktop.DBus.Error.MatchRuleInvalid"
#define DBUS_ERROR_SPAWN_EXEC_FAILED        "org.freedesktop.DBus.Error.Spawn.ExecFailed"
#define DBUS_ERROR_SPAWN_FORK_FAILED        "org.freedesktop.DBus.Error.Spawn.ForkFailed"
#define DBUS_ERROR_SPAWN_CHILD_EXITED       "org.freedesktop.DBus.Error.Spawn.ChildExited"
#define DBUS_ERROR_SPAWN_CHILD_SIGNALED     "org.freedesktop.DBus.Error.Spawn.ChildSignaled"
#define DBUS_ERROR_SPAWN_FAILED             "org.freedesktop.DBus.Error.Spawn.Failed"
#define DBUS_ERROR_SPAWN_SETUP_FAILED       "org.freedesktop.DBus.Error.Spawn.FailedToSetup"
#define DBUS_ERROR_SPAWN_CONFIG_INVALID     "org.freedesktop.DBus.Error.Spawn.ConfigInvalid"
#define DBUS_ERROR_SPAWN_SERVICE_INVALID    "org.freedesktop.DBus.Error.Spawn.ServiceNotValid"
#define DBUS_ERROR_SPAWN_SERVICE_NOT_FOUND  "org.freedesktop.DBus.Error.Spawn.ServiceNotFound"
#define DBUS_ERROR_SPAWN_PERMISSIONS_INVALID "org.freedesktop.DBus.Error.Spawn.PermissionsInvalid"
#define DBUS_ERROR_SPAWN_FILE_INVALID       "org.freedesktop.DBus.Error.Spawn.FileInvalid"
#define DBUS_ERROR_SPAWN_NO_MEMORY          "org.freedesktop.DBus.Error.Spawn.NoMemory"
#define DBUS_ERROR_UNIX_PROCESS_ID_UNKNOWN  "org.freedesktop.DBus.Error.UnixProcessIdUnknown"
#define DBUS_ERROR_INVALID_SIGNATURE        "org.freedesktop.DBus.Error.InvalidSignature"
#define DBUS_ERROR_INVALID_FILE_CONTENT     "org.freedesktop.DBus.Error.InvalidFileContent"
#define DBUS_ERROR_SELINUX_SECURITY_CONTEXT_UNKNOWN "org.freedesktop.DBus.Error.SELinuxSecurityContextUnknown"
#define DBUS_ERROR_ADT_AUDIT_DATA_UNKNOWN   "org.freedesktop.DBus.Error.AdtAuditDataUnknown"
#define DBUS_ERROR_OBJECT_PATH_IN_USE       "org.freedesktop.DBus.Error.ObjectPathInUse"
#define DBUS_ERROR_INCONSISTENT_MESSAGE     "org.freedesktop.DBus.Error.InconsistentMessage"
#define DBUS_ERROR_INTERACTIVE_AUTHORIZATION_REQUIRED "org.freedesktop.DBus.Error.InteractiveAuthorizationRequired"
#define DBUS_ERROR_NOT_CONTAINER            "org.freedesktop.DBus.Error.NotContainer"

#endif /* _DBUS_DBUS_PROTOCOL_H */
