/*
 * VeridianOS -- kio-veridian-worker.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * KIO worker for local filesystem access on VeridianOS.  Implements the
 * file:// protocol for the KDE I/O framework, providing filesystem
 * operations (stat, list, get, put, mkdir, rename, delete, copy, chmod)
 * via POSIX APIs.  Also provides a basic trash:// protocol stub.
 */

#ifndef KIO_VERIDIAN_WORKER_H
#define KIO_VERIDIAN_WORKER_H

#include <KIO/WorkerBase>

#include <QString>
#include <QUrl>
#include <QDateTime>

/**
 * KIO worker for local file operations on VeridianOS.
 *
 * This worker handles the file:// protocol, mapping KIO operations to
 * POSIX system calls.  It serves as the primary filesystem access layer
 * for KDE applications (Dolphin, Kate, etc.) on VeridianOS.
 *
 * Supported operations:
 *   - stat:    Query file metadata (size, permissions, timestamps)
 *   - listDir: Enumerate directory contents
 *   - get:     Read file data (streaming)
 *   - put:     Write file data (streaming)
 *   - mkdir:   Create directories (with parents)
 *   - rename:  Move/rename files and directories
 *   - del:     Delete files and directories
 *   - copy:    Copy files (with progress reporting)
 *   - chmod:   Change file permissions
 *
 * Also provides a basic trash:// protocol that moves files to
 * ~/.local/share/Trash/ following the freedesktop.org Trash spec.
 */
class VeridianFileWorker : public KIO::WorkerBase
{
public:
    VeridianFileWorker(const QByteArray &pool, const QByteArray &app);
    ~VeridianFileWorker() override;

    /* ===================================================================== */
    /* KIO::WorkerBase virtual method overrides                              */
    /* ===================================================================== */

    /**
     * Query file/directory metadata.
     *
     * Uses POSIX stat() to populate a KIO::UDSEntry with:
     *   UDS_NAME, UDS_SIZE, UDS_MODIFICATION_TIME, UDS_ACCESS_TIME,
     *   UDS_CREATION_TIME, UDS_FILE_TYPE, UDS_ACCESS (permissions),
     *   UDS_USER, UDS_GROUP, UDS_MIME_TYPE, UDS_LINK_DEST (symlinks)
     */
    KIO::WorkerResult stat(const QUrl &url) override;

    /**
     * List directory contents.
     *
     * Uses POSIX opendir/readdir to enumerate entries.  Each entry is
     * stat'd to provide full metadata.  Emits listEntry() for each
     * entry found.
     */
    KIO::WorkerResult listDir(const QUrl &url) override;

    /**
     * Read file contents (streaming).
     *
     * Uses POSIX open/read to stream file data to the caller via
     * data() calls.  Supports resume (offset) if the file supports
     * seeking.
     */
    KIO::WorkerResult get(const QUrl &url) override;

    /**
     * Write file contents (streaming).
     *
     * Uses POSIX open/write to write data received from the caller.
     * Supports overwrite, resume, and permission setting.
     */
    KIO::WorkerResult put(const QUrl &url, int permissions,
                          KIO::JobFlags flags) override;

    /**
     * Create a directory.
     *
     * Uses POSIX mkdir().  Creates parent directories as needed if
     * they don't exist.
     */
    KIO::WorkerResult mkdir(const QUrl &url, int permissions) override;

    /**
     * Rename/move a file or directory.
     *
     * Uses POSIX rename().  Falls back to copy+delete if rename()
     * fails with EXDEV (cross-device move).
     */
    KIO::WorkerResult rename(const QUrl &src, const QUrl &dest,
                             KIO::JobFlags flags) override;

    /**
     * Delete a file or directory.
     *
     * Uses POSIX unlink() for files and rmdir() for directories.
     * Recursive deletion is handled by the KIO framework.
     */
    KIO::WorkerResult del(const QUrl &url, bool isFile) override;

    /**
     * Copy a file.
     *
     * Uses POSIX open/read/write to copy file data with progress
     * reporting.  Preserves permissions and timestamps.
     */
    KIO::WorkerResult copy(const QUrl &src, const QUrl &dest,
                           int permissions, KIO::JobFlags flags) override;

    /**
     * Change file permissions.
     *
     * Uses POSIX chmod().
     */
    KIO::WorkerResult chmod(const QUrl &url, int permissions) override;

private:
    /* ===================================================================== */
    /* Internal helpers                                                       */
    /* ===================================================================== */

    /**
     * Build a KIO::UDSEntry from a POSIX stat result.
     */
    KIO::UDSEntry createUDSEntry(const QString &name, const QString &path);

    /**
     * Detect MIME type from file extension.
     *
     * Simple extension-based detection covering common file types.
     * Falls back to "application/octet-stream" for unknown extensions
     * and "inode/directory" for directories.
     */
    QString mimeTypeForFile(const QString &path) const;

    /**
     * Move a file to the trash.
     *
     * Implements the freedesktop.org Trash specification:
     *   - Moves file to ~/.local/share/Trash/files/
     *   - Creates .trashinfo metadata in ~/.local/share/Trash/info/
     *   - Handles name collisions with numeric suffixes
     */
    KIO::WorkerResult moveToTrash(const QString &path);

    /**
     * Get the trash directory path, creating it if needed.
     */
    QString trashDirectory() const;

    /**
     * Convert POSIX errno to a KIO error code.
     */
    int errnoToKioError(int posixErrno) const;

    /**
     * Resolve a QUrl to a local filesystem path.
     * Handles file:// URLs and trash:// URLs.
     */
    QString urlToLocalPath(const QUrl &url) const;
};

#endif /* KIO_VERIDIAN_WORKER_H */
