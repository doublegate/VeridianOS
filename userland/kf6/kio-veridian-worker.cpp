/*
 * VeridianOS -- kio-veridian-worker.cpp
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * KIO worker implementation for local filesystem access on VeridianOS.
 * Handles file:// and trash:// protocols using POSIX APIs.
 */

#include "kio-veridian-worker.h"

#include <KIO/UDSEntry>
#include <KIO/Global>

#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QMimeDatabase>
#include <QStandardPaths>
#include <QDateTime>
#include <QDebug>

#include <sys/types.h>
#include <sys/stat.h>
#include <dirent.h>
#include <fcntl.h>
#include <unistd.h>
#include <string.h>
#include <errno.h>
#include <pwd.h>
#include <grp.h>

/* Read/write buffer size for file transfers */
static constexpr int TRANSFER_BUFFER_SIZE = 65536;

/* ========================================================================= */
/* Construction / destruction                                                */
/* ========================================================================= */

VeridianFileWorker::VeridianFileWorker(const QByteArray &pool,
                                       const QByteArray &app)
    : KIO::WorkerBase(QByteArrayLiteral("file"), pool, app)
{
}

VeridianFileWorker::~VeridianFileWorker()
{
}

/* ========================================================================= */
/* stat                                                                      */
/* ========================================================================= */

KIO::WorkerResult VeridianFileWorker::stat(const QUrl &url)
{
    const QString path = urlToLocalPath(url);
    if (path.isEmpty())
        return KIO::WorkerResult::fail(KIO::ERR_MALFORMED_URL,
                                       url.toDisplayString());

    struct stat st;
    if (::lstat(path.toLocal8Bit().constData(), &st) < 0) {
        return KIO::WorkerResult::fail(errnoToKioError(errno), path);
    }

    KIO::UDSEntry entry = createUDSEntry(
        QFileInfo(path).fileName(), path);

    statEntry(entry);
    return KIO::WorkerResult::pass();
}

/* ========================================================================= */
/* listDir                                                                   */
/* ========================================================================= */

KIO::WorkerResult VeridianFileWorker::listDir(const QUrl &url)
{
    const QString path = urlToLocalPath(url);
    if (path.isEmpty())
        return KIO::WorkerResult::fail(KIO::ERR_MALFORMED_URL,
                                       url.toDisplayString());

    DIR *dp = ::opendir(path.toLocal8Bit().constData());
    if (!dp) {
        return KIO::WorkerResult::fail(errnoToKioError(errno), path);
    }

    struct dirent *de;
    while ((de = ::readdir(dp)) != nullptr) {
        /* Skip . and .. */
        if (strcmp(de->d_name, ".") == 0 || strcmp(de->d_name, "..") == 0)
            continue;

        const QString name = QString::fromLocal8Bit(de->d_name);
        const QString entryPath = path + QLatin1Char('/') + name;

        KIO::UDSEntry entry = createUDSEntry(name, entryPath);
        listEntry(entry);
    }

    ::closedir(dp);
    return KIO::WorkerResult::pass();
}

/* ========================================================================= */
/* get (read file)                                                           */
/* ========================================================================= */

KIO::WorkerResult VeridianFileWorker::get(const QUrl &url)
{
    const QString path = urlToLocalPath(url);
    if (path.isEmpty())
        return KIO::WorkerResult::fail(KIO::ERR_MALFORMED_URL,
                                       url.toDisplayString());

    int fd = ::open(path.toLocal8Bit().constData(), O_RDONLY);
    if (fd < 0) {
        return KIO::WorkerResult::fail(errnoToKioError(errno), path);
    }

    /* Get file size for progress reporting */
    struct stat st;
    if (::fstat(fd, &st) == 0) {
        totalSize(st.st_size);
    }

    /* Set MIME type */
    mimeType(mimeTypeForFile(path));

    /* Stream file data */
    char buf[TRANSFER_BUFFER_SIZE];
    KIO::filesize_t totalRead = 0;

    for (;;) {
        ssize_t n = ::read(fd, buf, sizeof(buf));
        if (n < 0) {
            int savedErrno = errno;
            ::close(fd);
            return KIO::WorkerResult::fail(errnoToKioError(savedErrno), path);
        }
        if (n == 0)
            break;

        data(QByteArray(buf, static_cast<int>(n)));
        totalRead += static_cast<KIO::filesize_t>(n);
        processedSize(totalRead);
    }

    /* Signal end of data */
    data(QByteArray());
    ::close(fd);
    return KIO::WorkerResult::pass();
}

/* ========================================================================= */
/* put (write file)                                                          */
/* ========================================================================= */

KIO::WorkerResult VeridianFileWorker::put(const QUrl &url, int permissions,
                                           KIO::JobFlags flags)
{
    const QString path = urlToLocalPath(url);
    if (path.isEmpty())
        return KIO::WorkerResult::fail(KIO::ERR_MALFORMED_URL,
                                       url.toDisplayString());

    /* Check if file exists and handle overwrite flag */
    struct stat st;
    bool exists = (::stat(path.toLocal8Bit().constData(), &st) == 0);

    if (exists && !(flags & KIO::Overwrite)) {
        return KIO::WorkerResult::fail(KIO::ERR_FILE_ALREADY_EXIST, path);
    }

    /* Open file for writing */
    int openFlags = O_WRONLY | O_CREAT | O_TRUNC;
    mode_t mode = (permissions > 0)
                      ? static_cast<mode_t>(permissions)
                      : (S_IRUSR | S_IWUSR | S_IRGRP | S_IROTH);

    int fd = ::open(path.toLocal8Bit().constData(), openFlags, mode);
    if (fd < 0) {
        return KIO::WorkerResult::fail(errnoToKioError(errno), path);
    }

    /* Read data from job and write to file */
    int dataResult;
    do {
        QByteArray buffer;
        dataReq();
        dataResult = readData(buffer);
        if (dataResult < 0) {
            ::close(fd);
            return KIO::WorkerResult::fail(KIO::ERR_CANNOT_WRITE, path);
        }
        if (buffer.isEmpty())
            break;

        const char *ptr = buffer.constData();
        qint64 remaining = buffer.size();
        while (remaining > 0) {
            ssize_t written = ::write(fd, ptr, static_cast<size_t>(remaining));
            if (written < 0) {
                int savedErrno = errno;
                ::close(fd);
                return KIO::WorkerResult::fail(
                    errnoToKioError(savedErrno), path);
            }
            ptr += written;
            remaining -= written;
        }
    } while (dataResult > 0);

    ::close(fd);
    return KIO::WorkerResult::pass();
}

/* ========================================================================= */
/* mkdir                                                                     */
/* ========================================================================= */

KIO::WorkerResult VeridianFileWorker::mkdir(const QUrl &url, int permissions)
{
    const QString path = urlToLocalPath(url);
    if (path.isEmpty())
        return KIO::WorkerResult::fail(KIO::ERR_MALFORMED_URL,
                                       url.toDisplayString());

    mode_t mode = (permissions > 0)
                      ? static_cast<mode_t>(permissions)
                      : (S_IRWXU | S_IRGRP | S_IXGRP | S_IROTH | S_IXOTH);

    if (::mkdir(path.toLocal8Bit().constData(), mode) < 0) {
        if (errno == EEXIST) {
            /* Check if it's already a directory */
            struct stat st;
            if (::stat(path.toLocal8Bit().constData(), &st) == 0 &&
                S_ISDIR(st.st_mode)) {
                return KIO::WorkerResult::pass();
            }
        }
        return KIO::WorkerResult::fail(errnoToKioError(errno), path);
    }

    return KIO::WorkerResult::pass();
}

/* ========================================================================= */
/* rename                                                                    */
/* ========================================================================= */

KIO::WorkerResult VeridianFileWorker::rename(const QUrl &src, const QUrl &dest,
                                              KIO::JobFlags flags)
{
    const QString srcPath = urlToLocalPath(src);
    const QString destPath = urlToLocalPath(dest);

    if (srcPath.isEmpty() || destPath.isEmpty())
        return KIO::WorkerResult::fail(KIO::ERR_MALFORMED_URL,
                                       src.toDisplayString());

    /* Check destination exists */
    struct stat st;
    if (::stat(destPath.toLocal8Bit().constData(), &st) == 0) {
        if (!(flags & KIO::Overwrite)) {
            return KIO::WorkerResult::fail(KIO::ERR_FILE_ALREADY_EXIST,
                                           destPath);
        }
    }

    if (::rename(srcPath.toLocal8Bit().constData(),
                 destPath.toLocal8Bit().constData()) < 0) {
        if (errno == EXDEV) {
            /* Cross-device: fall back to copy + delete */
            KIO::WorkerResult result = copy(src, dest, -1, flags);
            if (result.success()) {
                ::unlink(srcPath.toLocal8Bit().constData());
            }
            return result;
        }
        return KIO::WorkerResult::fail(errnoToKioError(errno), srcPath);
    }

    return KIO::WorkerResult::pass();
}

/* ========================================================================= */
/* del (delete)                                                              */
/* ========================================================================= */

KIO::WorkerResult VeridianFileWorker::del(const QUrl &url, bool isFile)
{
    const QString path = urlToLocalPath(url);
    if (path.isEmpty())
        return KIO::WorkerResult::fail(KIO::ERR_MALFORMED_URL,
                                       url.toDisplayString());

    /* Handle trash:// URLs by restoring or just deleting */
    if (url.scheme() == QStringLiteral("trash")) {
        /* For trash, just delete the actual file */
    }

    int ret;
    if (isFile) {
        ret = ::unlink(path.toLocal8Bit().constData());
    } else {
        ret = ::rmdir(path.toLocal8Bit().constData());
    }

    if (ret < 0) {
        return KIO::WorkerResult::fail(errnoToKioError(errno), path);
    }

    return KIO::WorkerResult::pass();
}

/* ========================================================================= */
/* copy                                                                      */
/* ========================================================================= */

KIO::WorkerResult VeridianFileWorker::copy(const QUrl &src, const QUrl &dest,
                                            int permissions,
                                            KIO::JobFlags flags)
{
    const QString srcPath = urlToLocalPath(src);
    const QString destPath = urlToLocalPath(dest);

    if (srcPath.isEmpty() || destPath.isEmpty())
        return KIO::WorkerResult::fail(KIO::ERR_MALFORMED_URL,
                                       src.toDisplayString());

    /* Check destination */
    struct stat destSt;
    if (::stat(destPath.toLocal8Bit().constData(), &destSt) == 0) {
        if (!(flags & KIO::Overwrite)) {
            return KIO::WorkerResult::fail(KIO::ERR_FILE_ALREADY_EXIST,
                                           destPath);
        }
    }

    /* Open source */
    int srcFd = ::open(srcPath.toLocal8Bit().constData(), O_RDONLY);
    if (srcFd < 0) {
        return KIO::WorkerResult::fail(errnoToKioError(errno), srcPath);
    }

    /* Get source file info */
    struct stat srcSt;
    if (::fstat(srcFd, &srcSt) < 0) {
        int savedErrno = errno;
        ::close(srcFd);
        return KIO::WorkerResult::fail(errnoToKioError(savedErrno), srcPath);
    }
    totalSize(srcSt.st_size);

    /* Open destination */
    mode_t mode = (permissions > 0)
                      ? static_cast<mode_t>(permissions)
                      : (srcSt.st_mode & 07777);
    int destFd = ::open(destPath.toLocal8Bit().constData(),
                        O_WRONLY | O_CREAT | O_TRUNC, mode);
    if (destFd < 0) {
        int savedErrno = errno;
        ::close(srcFd);
        return KIO::WorkerResult::fail(errnoToKioError(savedErrno), destPath);
    }

    /* Copy data */
    char buf[TRANSFER_BUFFER_SIZE];
    KIO::filesize_t totalCopied = 0;

    for (;;) {
        ssize_t n = ::read(srcFd, buf, sizeof(buf));
        if (n < 0) {
            int savedErrno = errno;
            ::close(srcFd);
            ::close(destFd);
            return KIO::WorkerResult::fail(errnoToKioError(savedErrno),
                                           srcPath);
        }
        if (n == 0)
            break;

        const char *ptr = buf;
        ssize_t remaining = n;
        while (remaining > 0) {
            ssize_t written = ::write(destFd, ptr,
                                      static_cast<size_t>(remaining));
            if (written < 0) {
                int savedErrno = errno;
                ::close(srcFd);
                ::close(destFd);
                return KIO::WorkerResult::fail(
                    errnoToKioError(savedErrno), destPath);
            }
            ptr += written;
            remaining -= written;
        }

        totalCopied += static_cast<KIO::filesize_t>(n);
        processedSize(totalCopied);
    }

    ::close(srcFd);
    ::close(destFd);

    return KIO::WorkerResult::pass();
}

/* ========================================================================= */
/* chmod                                                                     */
/* ========================================================================= */

KIO::WorkerResult VeridianFileWorker::chmod(const QUrl &url, int permissions)
{
    const QString path = urlToLocalPath(url);
    if (path.isEmpty())
        return KIO::WorkerResult::fail(KIO::ERR_MALFORMED_URL,
                                       url.toDisplayString());

    if (::chmod(path.toLocal8Bit().constData(),
                static_cast<mode_t>(permissions)) < 0) {
        return KIO::WorkerResult::fail(errnoToKioError(errno), path);
    }

    return KIO::WorkerResult::pass();
}

/* ========================================================================= */
/* Internal helpers                                                          */
/* ========================================================================= */

KIO::UDSEntry VeridianFileWorker::createUDSEntry(const QString &name,
                                                   const QString &path)
{
    KIO::UDSEntry entry;
    struct stat st;

    if (::lstat(path.toLocal8Bit().constData(), &st) < 0) {
        entry.fastInsert(KIO::UDSEntry::UDS_NAME, name);
        return entry;
    }

    entry.fastInsert(KIO::UDSEntry::UDS_NAME, name);
    entry.fastInsert(KIO::UDSEntry::UDS_SIZE,
                     static_cast<long long>(st.st_size));
    entry.fastInsert(KIO::UDSEntry::UDS_MODIFICATION_TIME,
                     static_cast<long long>(st.st_mtime));
    entry.fastInsert(KIO::UDSEntry::UDS_ACCESS_TIME,
                     static_cast<long long>(st.st_atime));
    entry.fastInsert(KIO::UDSEntry::UDS_CREATION_TIME,
                     static_cast<long long>(st.st_ctime));
    entry.fastInsert(KIO::UDSEntry::UDS_FILE_TYPE,
                     static_cast<long long>(st.st_mode & S_IFMT));
    entry.fastInsert(KIO::UDSEntry::UDS_ACCESS,
                     static_cast<long long>(st.st_mode & 07777));

    /* User and group names */
    struct passwd *pw = ::getpwuid(st.st_uid);
    if (pw) {
        entry.fastInsert(KIO::UDSEntry::UDS_USER,
                         QString::fromLocal8Bit(pw->pw_name));
    }
    struct group *gr = ::getgrgid(st.st_gid);
    if (gr) {
        entry.fastInsert(KIO::UDSEntry::UDS_GROUP,
                         QString::fromLocal8Bit(gr->gr_name));
    }

    /* MIME type */
    if (S_ISDIR(st.st_mode)) {
        entry.fastInsert(KIO::UDSEntry::UDS_MIME_TYPE,
                         QStringLiteral("inode/directory"));
    } else {
        entry.fastInsert(KIO::UDSEntry::UDS_MIME_TYPE,
                         mimeTypeForFile(path));
    }

    /* Symlink target */
    if (S_ISLNK(st.st_mode)) {
        char linkTarget[PATH_MAX];
        ssize_t len = ::readlink(path.toLocal8Bit().constData(),
                                 linkTarget, sizeof(linkTarget) - 1);
        if (len > 0) {
            linkTarget[len] = '\0';
            entry.fastInsert(KIO::UDSEntry::UDS_LINK_DEST,
                             QString::fromLocal8Bit(linkTarget));
        }
    }

    return entry;
}

QString VeridianFileWorker::mimeTypeForFile(const QString &path) const
{
    /* Extension-based MIME type detection for common types */
    static const struct {
        const char *ext;
        const char *mime;
    } mimeMap[] = {
        { ".txt",    "text/plain" },
        { ".html",   "text/html" },
        { ".htm",    "text/html" },
        { ".css",    "text/css" },
        { ".js",     "application/javascript" },
        { ".json",   "application/json" },
        { ".xml",    "application/xml" },
        { ".svg",    "image/svg+xml" },
        { ".png",    "image/png" },
        { ".jpg",    "image/jpeg" },
        { ".jpeg",   "image/jpeg" },
        { ".gif",    "image/gif" },
        { ".bmp",    "image/bmp" },
        { ".webp",   "image/webp" },
        { ".ico",    "image/x-icon" },
        { ".pdf",    "application/pdf" },
        { ".zip",    "application/zip" },
        { ".tar",    "application/x-tar" },
        { ".gz",     "application/gzip" },
        { ".bz2",    "application/x-bzip2" },
        { ".xz",     "application/x-xz" },
        { ".7z",     "application/x-7z-compressed" },
        { ".deb",    "application/x-deb" },
        { ".rpm",    "application/x-rpm" },
        { ".sh",     "application/x-shellscript" },
        { ".py",     "text/x-python" },
        { ".rs",     "text/x-rust" },
        { ".c",      "text/x-csrc" },
        { ".cpp",    "text/x-c++src" },
        { ".h",      "text/x-chdr" },
        { ".hpp",    "text/x-c++hdr" },
        { ".o",      "application/x-object" },
        { ".so",     "application/x-sharedlib" },
        { ".a",      "application/x-archive" },
        { ".mp3",    "audio/mpeg" },
        { ".ogg",    "audio/ogg" },
        { ".wav",    "audio/wav" },
        { ".mp4",    "video/mp4" },
        { ".mkv",    "video/x-matroska" },
        { ".avi",    "video/x-msvideo" },
        { ".desktop","application/x-desktop" },
        { nullptr,   nullptr }
    };

    const QString lowerPath = path.toLower();
    for (int i = 0; mimeMap[i].ext != nullptr; ++i) {
        if (lowerPath.endsWith(QLatin1String(mimeMap[i].ext)))
            return QString::fromLatin1(mimeMap[i].mime);
    }

    return QStringLiteral("application/octet-stream");
}

KIO::WorkerResult VeridianFileWorker::moveToTrash(const QString &path)
{
    const QString trashDir = trashDirectory();
    const QString filesDir = trashDir + QStringLiteral("/files");
    const QString infoDir = trashDir + QStringLiteral("/info");

    /* Ensure trash directories exist */
    ::mkdir(trashDir.toLocal8Bit().constData(), 0700);
    ::mkdir(filesDir.toLocal8Bit().constData(), 0700);
    ::mkdir(infoDir.toLocal8Bit().constData(), 0700);

    /* Generate unique name in trash */
    QFileInfo fi(path);
    QString baseName = fi.fileName();
    QString trashPath = filesDir + QLatin1Char('/') + baseName;
    QString infoPath = infoDir + QLatin1Char('/') + baseName +
                       QStringLiteral(".trashinfo");

    int suffix = 1;
    while (QFileInfo::exists(trashPath)) {
        trashPath = filesDir + QLatin1Char('/') + fi.completeBaseName() +
                    QStringLiteral(".") + QString::number(suffix);
        if (!fi.suffix().isEmpty())
            trashPath += QLatin1Char('.') + fi.suffix();
        infoPath = infoDir + QLatin1Char('/') + fi.completeBaseName() +
                   QStringLiteral(".") + QString::number(suffix) +
                   QStringLiteral(".trashinfo");
        ++suffix;
    }

    /* Move file to trash */
    if (::rename(path.toLocal8Bit().constData(),
                 trashPath.toLocal8Bit().constData()) < 0) {
        return KIO::WorkerResult::fail(errnoToKioError(errno), path);
    }

    /* Write .trashinfo metadata */
    QFile info(infoPath);
    if (info.open(QIODevice::WriteOnly)) {
        QTextStream ts(&info);
        ts << "[Trash Info]\n";
        ts << "Path=" << path << "\n";
        ts << "DeletionDate=" <<
            QDateTime::currentDateTime().toString(Qt::ISODate) << "\n";
        info.close();
    }

    return KIO::WorkerResult::pass();
}

QString VeridianFileWorker::trashDirectory() const
{
    QString dataHome = QStandardPaths::writableLocation(
        QStandardPaths::GenericDataLocation);
    if (dataHome.isEmpty())
        dataHome = QDir::homePath() + QStringLiteral("/.local/share");
    return dataHome + QStringLiteral("/Trash");
}

int VeridianFileWorker::errnoToKioError(int posixErrno) const
{
    switch (posixErrno) {
    case EACCES:
    case EPERM:
        return KIO::ERR_ACCESS_DENIED;
    case ENOENT:
        return KIO::ERR_DOES_NOT_EXIST;
    case EEXIST:
        return KIO::ERR_FILE_ALREADY_EXIST;
    case ENOTDIR:
        return KIO::ERR_IS_FILE;
    case EISDIR:
        return KIO::ERR_IS_DIRECTORY;
    case ENOSPC:
        return KIO::ERR_DISK_FULL;
    case EROFS:
        return KIO::ERR_WRITE_ACCESS_DENIED;
    case ENAMETOOLONG:
        return KIO::ERR_MALFORMED_URL;
    case ENOTEMPTY:
        return KIO::ERR_CANNOT_RMDIR;
    default:
        return KIO::ERR_UNKNOWN;
    }
}

QString VeridianFileWorker::urlToLocalPath(const QUrl &url) const
{
    if (url.scheme() == QStringLiteral("trash")) {
        /* Map trash:// to ~/.local/share/Trash/files/ */
        QString trashPath = url.path();
        if (trashPath.startsWith(QLatin1Char('/')))
            trashPath = trashPath.mid(1);
        return trashDirectory() + QStringLiteral("/files/") + trashPath;
    }

    /* file:// or empty scheme -> local path */
    return url.toLocalFile();
}
