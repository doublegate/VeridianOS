/*
 * VeridianOS -- baloo-veridian-backend.cpp
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Baloo file indexer backend implementation for VeridianOS.
 *
 * Features:
 *   - Recursive directory crawler with rate limiting (100 files/batch)
 *   - Filename tokenization: split on . - _ space and camelCase boundaries
 *   - Content extraction for text files (first 64KB, skip binary)
 *   - Inotify-based directory watching for live updates
 *   - Persistent index via BLIX binary format (baloo-veridian-index)
 *   - Periodic auto-save (every 5 minutes or 1000 new files)
 *   - D-Bus service at org.kde.baloo for status queries
 *
 * Excluded by default: hidden files, /proc, /sys, /dev, user-specified paths.
 */

#include "baloo-veridian-backend.h"
#include "baloo-veridian-index.h"

#include <QDebug>
#include <QDir>
#include <QDirIterator>
#include <QFile>
#include <QFileInfo>
#include <QMimeDatabase>
#include <QMimeType>
#include <QTimer>
#include <QDBusConnection>
#include <QDBusMessage>
#include <QDateTime>
#include <QSet>
#include <QQueue>
#include <QElapsedTimer>

#include <string.h>
#include <sys/inotify.h>
#include <sys/stat.h>
#include <unistd.h>
#include <fcntl.h>
#include <errno.h>
#include <poll.h>

namespace Baloo {

/* ========================================================================= */
/* Configuration                                                             */
/* ========================================================================= */

static const int CRAWL_BATCH_SIZE       = 100;
static const int CONTENT_MAX_BYTES      = 65536;  /* 64 KB */
static const int BINARY_CHECK_BYTES     = 512;
static const int AUTOSAVE_INTERVAL_MS   = 300000;  /* 5 minutes */
static const int AUTOSAVE_FILE_COUNT    = 1000;
static const int INOTIFY_BUF_SIZE       = 8192;

/* Text file extensions eligible for content indexing */
static const char *TEXT_EXTENSIONS[] = {
    ".txt", ".md", ".rs", ".cpp", ".c", ".h", ".hpp",
    ".py", ".sh", ".json", ".xml", ".html", ".css", ".js",
    ".toml", ".yaml", ".yml", ".cfg", ".conf", ".ini",
    ".cmake", ".mk", ".log",
    nullptr
};

/* ========================================================================= */
/* Backend state                                                             */
/* ========================================================================= */

static BalooIndex *s_index = nullptr;
static BalooIndexState s_state = BALOO_IDLE;
static QString s_indexPath;
static QStringList s_excludedPaths;
static QQueue<QString> s_crawlQueue;
static bool s_crawlActive = false;
static uint64_t s_filesProcessed = 0;
static uint64_t s_filesSinceLastSave = 0;
static QElapsedTimer s_lastSaveTimer;

/* Inotify state */
static int s_inotifyFd = -1;
static QHash<int, QString> s_watchDescriptors;  /* wd -> path */

/* ========================================================================= */
/* Default excluded paths                                                    */
/* ========================================================================= */

static const char *DEFAULT_EXCLUDED[] = {
    "/proc", "/sys", "/dev", "/run", "/tmp",
    "/var/cache", "/var/tmp", "/var/log",
    nullptr
};

/* ========================================================================= */
/* Helpers                                                                   */
/* ========================================================================= */

static bool isHidden(const QString &path)
{
    /* Check if any component starts with '.' */
    QStringList parts = path.split(QLatin1Char('/'), Qt::SkipEmptyParts);
    for (const QString &part : parts) {
        if (part.startsWith(QLatin1Char('.')))
            return true;
    }
    return false;
}

static bool isExcluded(const QString &path)
{
    for (const QString &excl : s_excludedPaths) {
        if (path.startsWith(excl))
            return true;
    }
    return false;
}

static bool isBinaryFile(const QByteArray &data)
{
    int checkLen = qMin(data.size(), BINARY_CHECK_BYTES);
    for (int i = 0; i < checkLen; ++i) {
        if (data[i] == '\0')
            return true;
    }
    return false;
}

static bool isTextExtension(const QString &suffix)
{
    QByteArray s = ("." + suffix.toLower()).toUtf8();
    for (int i = 0; TEXT_EXTENSIONS[i]; ++i) {
        if (s == TEXT_EXTENSIONS[i])
            return true;
    }
    return false;
}

/* ========================================================================= */
/* Filename tokenization                                                     */
/* ========================================================================= */

static QStringList tokenizeFilename(const QString &filename)
{
    QStringList tokens;
    QString current;

    for (int i = 0; i < filename.size(); ++i) {
        QChar c = filename[i];

        if (c.isLetterOrNumber()) {
            /* CamelCase split */
            if (c.isUpper() && !current.isEmpty() &&
                current[current.size() - 1].isLower()) {
                tokens.append(current.toLower());
                current.clear();
            }
            current.append(c);
        } else {
            /* Delimiter: . - _ space etc. */
            if (!current.isEmpty()) {
                tokens.append(current.toLower());
                current.clear();
            }
        }
    }

    if (!current.isEmpty())
        tokens.append(current.toLower());

    return tokens;
}

/* ========================================================================= */
/* File indexing                                                              */
/* ========================================================================= */

static void indexSingleFile(const QString &filePath)
{
    if (!s_index)
        return;

    QFileInfo fi(filePath);
    if (!fi.exists() || !fi.isReadable())
        return;

    QByteArray pathUtf8 = filePath.toUtf8();

    /* Index filename tokens */
    QStringList nameTokens = tokenizeFilename(fi.fileName());
    for (const QString &token : nameTokens) {
        QByteArray tokenUtf8 = token.toUtf8();
        baloo_index_add(s_index, tokenUtf8.constData(),
                        pathUtf8.constData(), 80);
    }

    /* Also index the full basename as a single token */
    QByteArray basename = fi.completeBaseName().toLower().toUtf8();
    if (!basename.isEmpty())
        baloo_index_add(s_index, basename.constData(),
                        pathUtf8.constData(), 70);

    /* Content indexing for text files */
    if (fi.isFile() && isTextExtension(fi.suffix())) {
        QFile file(filePath);
        if (file.open(QIODevice::ReadOnly)) {
            QByteArray data = file.read(CONTENT_MAX_BYTES);

            if (!isBinaryFile(data)) {
                /* Tokenize content words */
                QString content = QString::fromUtf8(data);
                QSet<QString> seenWords;
                QString currentWord;

                for (int i = 0; i < content.size(); ++i) {
                    QChar c = content[i];
                    if (c.isLetterOrNumber()) {
                        currentWord.append(c);
                    } else {
                        if (currentWord.size() >= 3 && currentWord.size() <= 64) {
                            QString lower = currentWord.toLower();
                            if (!seenWords.contains(lower)) {
                                seenWords.insert(lower);
                                QByteArray wordUtf8 = lower.toUtf8();
                                baloo_index_add(s_index, wordUtf8.constData(),
                                                pathUtf8.constData(), 40);
                            }
                        }
                        currentWord.clear();
                    }
                }

                /* Handle last word */
                if (currentWord.size() >= 3 && currentWord.size() <= 64) {
                    QString lower = currentWord.toLower();
                    if (!seenWords.contains(lower)) {
                        QByteArray wordUtf8 = lower.toUtf8();
                        baloo_index_add(s_index, wordUtf8.constData(),
                                        pathUtf8.constData(), 40);
                    }
                }
            }
        }
    }

    s_filesProcessed++;
    s_filesSinceLastSave++;
}

/* ========================================================================= */
/* Crawler                                                                   */
/* ========================================================================= */

static void processCrawlBatch()
{
    if (!s_crawlActive || s_crawlQueue.isEmpty()) {
        if (s_crawlActive) {
            s_crawlActive = false;
            s_state = BALOO_IDLE;

            /* Save index after crawl completes */
            if (s_index && !s_indexPath.isEmpty()) {
                baloo_index_save(s_index, s_indexPath.toUtf8().constData());
            }

            qDebug("Baloo: crawl complete -- %llu files indexed",
                   static_cast<unsigned long long>(s_filesProcessed));
        }
        return;
    }

    int processed = 0;

    while (!s_crawlQueue.isEmpty() && processed < CRAWL_BATCH_SIZE) {
        QString dirPath = s_crawlQueue.dequeue();

        if (isHidden(dirPath) || isExcluded(dirPath))
            continue;

        QDir dir(dirPath);
        if (!dir.exists() || !dir.isReadable())
            continue;

        QFileInfoList entries = dir.entryInfoList(
            QDir::Files | QDir::Dirs | QDir::NoDotAndDotDot | QDir::NoSymLinks);

        for (const QFileInfo &fi : entries) {
            if (fi.isDir()) {
                QString subPath = fi.absoluteFilePath();
                if (!isHidden(subPath) && !isExcluded(subPath))
                    s_crawlQueue.enqueue(subPath);
            } else if (fi.isFile()) {
                indexSingleFile(fi.absoluteFilePath());
                ++processed;
            }
        }
    }

    /* Auto-save check */
    if (s_filesSinceLastSave >= AUTOSAVE_FILE_COUNT ||
        (s_lastSaveTimer.isValid() && s_lastSaveTimer.elapsed() >= AUTOSAVE_INTERVAL_MS)) {
        if (s_index && !s_indexPath.isEmpty()) {
            baloo_index_save(s_index, s_indexPath.toUtf8().constData());
            s_filesSinceLastSave = 0;
            s_lastSaveTimer.restart();
            qDebug("Baloo: auto-saved index (%llu files)",
                   static_cast<unsigned long long>(s_filesProcessed));
        }
    }

    /* Schedule next batch (yield to event loop) */
    QTimer::singleShot(0, processCrawlBatch);
}

/* ========================================================================= */
/* Inotify                                                                   */
/* ========================================================================= */

static void setupInotify()
{
    if (s_inotifyFd >= 0)
        return;

    s_inotifyFd = inotify_init1(IN_NONBLOCK | IN_CLOEXEC);
    if (s_inotifyFd < 0) {
        qWarning("Baloo: inotify_init1 failed: %s", strerror(errno));
        return;
    }

    qDebug("Baloo: inotify initialized (fd=%d)", s_inotifyFd);
}

static void addInotifyWatch(const QString &dirPath)
{
    if (s_inotifyFd < 0)
        return;

    QByteArray pathUtf8 = dirPath.toUtf8();
    int wd = inotify_add_watch(s_inotifyFd, pathUtf8.constData(),
                                IN_CREATE | IN_MODIFY | IN_DELETE |
                                IN_MOVED_FROM | IN_MOVED_TO);
    if (wd < 0) {
        /* Silently skip if watch limit is reached */
        if (errno != ENOSPC)
            qWarning("Baloo: inotify_add_watch(%s) failed: %s",
                     pathUtf8.constData(), strerror(errno));
        return;
    }

    s_watchDescriptors.insert(wd, dirPath);
}

static void processInotifyEvents()
{
    if (s_inotifyFd < 0)
        return;

    char buf[INOTIFY_BUF_SIZE]
        __attribute__((aligned(__alignof__(struct inotify_event))));

    while (true) {
        ssize_t len = read(s_inotifyFd, buf, sizeof(buf));
        if (len <= 0)
            break;

        const char *ptr = buf;
        while (ptr < buf + len) {
            const struct inotify_event *event =
                reinterpret_cast<const struct inotify_event *>(ptr);

            if (event->len > 0) {
                QString dirPath = s_watchDescriptors.value(event->wd);
                if (!dirPath.isEmpty()) {
                    QString filePath = dirPath + QLatin1Char('/') +
                                       QString::fromUtf8(event->name);

                    if (event->mask & (IN_CREATE | IN_MODIFY | IN_MOVED_TO)) {
                        /* Re-index the file */
                        QFileInfo fi(filePath);
                        if (fi.isFile()) {
                            indexSingleFile(filePath);
                            qDebug("Baloo: re-indexed %s", qPrintable(filePath));
                        } else if (fi.isDir()) {
                            /* New directory: add watch and crawl */
                            addInotifyWatch(filePath);
                            s_crawlQueue.enqueue(filePath);
                            if (!s_crawlActive) {
                                s_crawlActive = true;
                                s_state = BALOO_CRAWLING;
                                QTimer::singleShot(0, processCrawlBatch);
                            }
                        }
                    } else if (event->mask & (IN_DELETE | IN_MOVED_FROM)) {
                        /* Remove from index */
                        if (s_index) {
                            QByteArray pathUtf8 = filePath.toUtf8();
                            baloo_index_remove_file(s_index, pathUtf8.constData());
                            qDebug("Baloo: removed %s from index",
                                   qPrintable(filePath));
                        }
                    }
                }
            }

            ptr += sizeof(struct inotify_event) + event->len;
        }
    }
}

/* ========================================================================= */
/* D-Bus registration                                                        */
/* ========================================================================= */

static bool registerDBus()
{
    QDBusConnection bus = QDBusConnection::sessionBus();
    if (!bus.registerService(QStringLiteral("org.kde.baloo"))) {
        qWarning("Baloo: failed to register D-Bus service: %s",
                 qPrintable(bus.lastError().message()));
        return false;
    }
    qDebug("Baloo: D-Bus service registered at org.kde.baloo");
    return true;
}

} /* namespace Baloo */

/* ========================================================================= */
/* C API implementation                                                      */
/* ========================================================================= */

extern "C" {

int baloo_init(const char *index_path)
{
    if (Baloo::s_index)
        return 0;  /* Already initialized */

    Baloo::s_indexPath = QString::fromUtf8(index_path);

    /* Set up default excluded paths */
    Baloo::s_excludedPaths.clear();
    for (int i = 0; Baloo::DEFAULT_EXCLUDED[i]; ++i)
        Baloo::s_excludedPaths.append(QString::fromUtf8(Baloo::DEFAULT_EXCLUDED[i]));

    /* Try to load existing index */
    Baloo::s_index = baloo_index_load(index_path);
    if (!Baloo::s_index)
        Baloo::s_index = baloo_index_create(index_path);

    if (!Baloo::s_index) {
        qWarning("Baloo: failed to create index");
        return -1;
    }

    Baloo::s_state = BALOO_IDLE;
    Baloo::s_filesProcessed = baloo_index_get_count(Baloo::s_index);
    Baloo::s_filesSinceLastSave = 0;
    Baloo::s_lastSaveTimer.start();

    /* Initialize inotify */
    Baloo::setupInotify();

    /* Register D-Bus */
    Baloo::registerDBus();

    qDebug("Baloo: initialized (index_path=%s, existing_files=%llu)",
           index_path,
           static_cast<unsigned long long>(Baloo::s_filesProcessed));
    return 0;
}

void baloo_destroy(void)
{
    if (!Baloo::s_index)
        return;

    /* Stop crawl */
    Baloo::s_crawlActive = false;
    Baloo::s_crawlQueue.clear();

    /* Save index */
    if (!Baloo::s_indexPath.isEmpty())
        baloo_index_save(Baloo::s_index, Baloo::s_indexPath.toUtf8().constData());

    /* Close inotify */
    if (Baloo::s_inotifyFd >= 0) {
        close(Baloo::s_inotifyFd);
        Baloo::s_inotifyFd = -1;
    }
    Baloo::s_watchDescriptors.clear();

    /* Destroy index */
    baloo_index_destroy(Baloo::s_index);
    Baloo::s_index = nullptr;
    Baloo::s_state = BALOO_IDLE;

    /* Unregister D-Bus */
    QDBusConnection::sessionBus().unregisterService(
        QStringLiteral("org.kde.baloo"));

    qDebug("Baloo: destroyed");
}

void baloo_start_crawl(const char *root_path)
{
    if (!Baloo::s_index || !root_path)
        return;

    if (Baloo::s_state == BALOO_SUSPENDED) {
        qWarning("Baloo: cannot start crawl while suspended");
        return;
    }

    QString root = QString::fromUtf8(root_path);
    if (!QDir(root).exists()) {
        qWarning("Baloo: crawl root does not exist: %s", root_path);
        return;
    }

    Baloo::s_crawlQueue.clear();
    Baloo::s_crawlQueue.enqueue(root);
    Baloo::s_crawlActive = true;
    Baloo::s_state = BALOO_CRAWLING;
    Baloo::s_filesProcessed = 0;

    qDebug("Baloo: starting crawl from %s", root_path);

    /* Add inotify watch for root */
    Baloo::addInotifyWatch(root);

    /* Begin processing */
    QTimer::singleShot(0, Baloo::processCrawlBatch);
}

void baloo_stop_crawl(void)
{
    if (!Baloo::s_crawlActive)
        return;

    Baloo::s_crawlActive = false;
    Baloo::s_crawlQueue.clear();
    Baloo::s_state = BALOO_IDLE;

    qDebug("Baloo: crawl stopped (%llu files processed)",
           static_cast<unsigned long long>(Baloo::s_filesProcessed));
}

int baloo_query(const char *query_string,
                BalooQueryType type,
                BalooResult *results_out,
                int max_results)
{
    if (!Baloo::s_index || !query_string || !results_out || max_results <= 0)
        return -1;

    Q_UNUSED(type);  /* All query types use the same unified index for now */

    /* Delegate to index search */
    BalooIndexResult *indexResults = new BalooIndexResult[max_results];
    int count = baloo_index_search(Baloo::s_index, query_string,
                                   indexResults, max_results);

    /* Convert index results to full BalooResults with file metadata */
    int resultCount = 0;
    for (int i = 0; i < count && resultCount < max_results; ++i) {
        QFileInfo fi(QString::fromUtf8(indexResults[i].path));
        if (!fi.exists())
            continue;

        BalooResult &r = results_out[resultCount];
        memset(&r, 0, sizeof(r));

        /* Copy path */
        size_t pathLen = strlen(indexResults[i].path);
        if (pathLen >= sizeof(r.path))
            pathLen = sizeof(r.path) - 1;
        memcpy(r.path, indexResults[i].path, pathLen);
        r.path[pathLen] = '\0';

        /* Filename */
        QByteArray fnUtf8 = fi.fileName().toUtf8();
        size_t fnLen = static_cast<size_t>(fnUtf8.size());
        if (fnLen >= sizeof(r.filename))
            fnLen = sizeof(r.filename) - 1;
        memcpy(r.filename, fnUtf8.constData(), fnLen);
        r.filename[fnLen] = '\0';

        /* Metadata */
        r.mtime = static_cast<uint64_t>(fi.lastModified().toSecsSinceEpoch());
        r.size = static_cast<uint64_t>(fi.size());
        r.relevance = indexResults[i].relevance;

        /* Content snippet (if content query and text file) */
        if (type == BALOO_QUERY_CONTENT && fi.isFile() &&
            Baloo::isTextExtension(fi.suffix())) {
            QFile file(fi.absoluteFilePath());
            if (file.open(QIODevice::ReadOnly)) {
                QByteArray data = file.read(4096);
                QString content = QString::fromUtf8(data);
                QString query = QString::fromUtf8(query_string).toLower();

                int pos = content.toLower().indexOf(query);
                if (pos >= 0) {
                    int start = qMax(0, pos - 40);
                    int end = qMin(content.size(), pos + query.size() + 40);
                    QString snippet = content.mid(start, end - start).trimmed();
                    snippet.replace(QLatin1Char('\n'), QLatin1Char(' '));

                    QByteArray snippetUtf8 = snippet.toUtf8();
                    size_t sLen = static_cast<size_t>(snippetUtf8.size());
                    if (sLen >= sizeof(r.content_snippet))
                        sLen = sizeof(r.content_snippet) - 1;
                    memcpy(r.content_snippet, snippetUtf8.constData(), sLen);
                    r.content_snippet[sLen] = '\0';
                }
            }
        }

        ++resultCount;
    }

    delete[] indexResults;
    return resultCount;
}

BalooIndexState baloo_get_state(void)
{
    return Baloo::s_state;
}

uint64_t baloo_get_indexed_count(void)
{
    if (!Baloo::s_index)
        return 0;
    return baloo_index_get_count(Baloo::s_index);
}

void baloo_suspend(void)
{
    if (Baloo::s_state == BALOO_SUSPENDED)
        return;

    if (Baloo::s_crawlActive) {
        Baloo::s_crawlActive = false;
        /* Keep the queue for resume */
    }

    Baloo::s_state = BALOO_SUSPENDED;
    qDebug("Baloo: indexing suspended");
}

void baloo_resume(void)
{
    if (Baloo::s_state != BALOO_SUSPENDED)
        return;

    Baloo::s_state = BALOO_IDLE;

    /* Resume crawl if queue is non-empty */
    if (!Baloo::s_crawlQueue.isEmpty()) {
        Baloo::s_crawlActive = true;
        Baloo::s_state = BALOO_CRAWLING;
        QTimer::singleShot(0, Baloo::processCrawlBatch);
        qDebug("Baloo: resumed crawl (%d directories queued)",
               Baloo::s_crawlQueue.size());
    } else {
        qDebug("Baloo: resumed (idle)");
    }
}

void baloo_set_excluded_paths(const char **paths, int count)
{
    Baloo::s_excludedPaths.clear();

    /* Always include default exclusions */
    for (int i = 0; Baloo::DEFAULT_EXCLUDED[i]; ++i)
        Baloo::s_excludedPaths.append(QString::fromUtf8(Baloo::DEFAULT_EXCLUDED[i]));

    /* Add user exclusions */
    for (int i = 0; i < count && paths[i]; ++i)
        Baloo::s_excludedPaths.append(QString::fromUtf8(paths[i]));

    qDebug("Baloo: %d excluded paths configured",
           Baloo::s_excludedPaths.size());
}

void baloo_index_file(const char *path)
{
    if (!path)
        return;
    Baloo::indexSingleFile(QString::fromUtf8(path));
}

void baloo_remove_file(const char *path)
{
    if (!Baloo::s_index || !path)
        return;
    baloo_index_remove_file(Baloo::s_index, path);
}

void baloo_watch_directory(const char *path)
{
    if (!path)
        return;
    Baloo::addInotifyWatch(QString::fromUtf8(path));
}

} /* extern "C" */
