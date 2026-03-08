/*
 * VeridianOS -- baloo-veridian-index.cpp
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Inverted index implementation for the Baloo file indexer.
 *
 * Data structures:
 *   - Word map:    hash map from word (string) -> vector of (path, relevance)
 *   - Trigram map:  hash map from 3-char sequence -> set of words
 *
 * Binary format (BLIX v1):
 *   Header:   "BLIX" (4 bytes), version (uint32), word_count (uint32)
 *   Per word: word_len (uint16), word_bytes, path_count (uint32),
 *             (path_len (uint16), path_bytes, relevance (int32)) * count
 *
 * Compaction is triggered when deletion count exceeds 20% of entries.
 */

#include "baloo-veridian-index.h"

#include <QDebug>
#include <QDir>
#include <QFile>
#include <QHash>
#include <QSet>
#include <QString>
#include <QVector>
#include <QDataStream>

#include <string.h>
#include <algorithm>

/* ========================================================================= */
/* Internal structures                                                       */
/* ========================================================================= */

struct IndexEntry {
    QString path;
    int relevance;
};

struct BalooIndex {
    /* Word -> list of (path, relevance) entries */
    QHash<QString, QVector<IndexEntry>> wordMap;

    /* Trigram -> set of words containing that trigram */
    QHash<QString, QSet<QString>> trigramMap;

    /* Set of all indexed file paths */
    QSet<QString> allPaths;

    /* Compaction tracking */
    int deletionCount;
    int totalEntries;

    /* Storage path */
    QString storagePath;
};

static const char BLIX_MAGIC[4] = { 'B', 'L', 'I', 'X' };
static const uint32_t BLIX_VERSION = 1;
static const float COMPACTION_THRESHOLD = 0.20f;

/* ========================================================================= */
/* Trigram extraction                                                        */
/* ========================================================================= */

static QSet<QString> extractTrigrams(const QString &word)
{
    QSet<QString> trigrams;
    QString lower = word.toLower();

    if (lower.size() < 3) {
        /* For short words, use the word itself as a "trigram" */
        if (!lower.isEmpty())
            trigrams.insert(lower);
        return trigrams;
    }

    for (int i = 0; i <= lower.size() - 3; ++i)
        trigrams.insert(lower.mid(i, 3));

    return trigrams;
}

/* ========================================================================= */
/* Tokenization                                                              */
/* ========================================================================= */

static QStringList tokenize(const QString &text)
{
    QStringList tokens;
    QString current;

    for (int i = 0; i < text.size(); ++i) {
        QChar c = text[i];

        if (c.isLetterOrNumber()) {
            /* CamelCase boundary: insert split before uppercase if preceded
             * by lowercase */
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
/* Compaction                                                                */
/* ========================================================================= */

static void compactIndex(BalooIndex *index)
{
    if (index->totalEntries == 0)
        return;

    float ratio = static_cast<float>(index->deletionCount) /
                  static_cast<float>(index->totalEntries);
    if (ratio < COMPACTION_THRESHOLD)
        return;

    qDebug("Baloo/Index: compacting (deletions %d / total %d = %.1f%%)",
           index->deletionCount, index->totalEntries, ratio * 100.0f);

    /* Remove empty word entries */
    QHash<QString, QVector<IndexEntry>>::iterator it = index->wordMap.begin();
    while (it != index->wordMap.end()) {
        if (it.value().isEmpty()) {
            /* Remove trigram references */
            QSet<QString> trigrams = extractTrigrams(it.key());
            for (const QString &tri : trigrams) {
                auto triIt = index->trigramMap.find(tri);
                if (triIt != index->trigramMap.end()) {
                    triIt->remove(it.key());
                    if (triIt->isEmpty())
                        index->trigramMap.erase(triIt);
                }
            }
            it = index->wordMap.erase(it);
        } else {
            ++it;
        }
    }

    /* Recount */
    index->totalEntries = 0;
    for (auto it = index->wordMap.constBegin(); it != index->wordMap.constEnd(); ++it)
        index->totalEntries += it.value().size();
    index->deletionCount = 0;

    qDebug("Baloo/Index: compaction complete (%d entries remain)",
           index->totalEntries);
}

/* ========================================================================= */
/* C API implementation                                                      */
/* ========================================================================= */

extern "C" {

BalooIndex *baloo_index_create(const char *storage_path)
{
    BalooIndex *index = new BalooIndex;
    index->deletionCount = 0;
    index->totalEntries = 0;
    index->storagePath = QString::fromUtf8(storage_path);

    qDebug("Baloo/Index: created new empty index at %s", storage_path);
    return index;
}

BalooIndex *baloo_index_load(const char *storage_path)
{
    QString path = QString::fromUtf8(storage_path) +
                   QStringLiteral("/baloo-index.blix");

    QFile file(path);
    if (!file.open(QIODevice::ReadOnly)) {
        qDebug("Baloo/Index: no saved index at %s", qPrintable(path));
        return nullptr;
    }

    QDataStream stream(&file);
    stream.setByteOrder(QDataStream::LittleEndian);

    /* Read and verify header */
    char magic[4];
    stream.readRawData(magic, 4);
    if (memcmp(magic, BLIX_MAGIC, 4) != 0) {
        qWarning("Baloo/Index: invalid magic in %s", qPrintable(path));
        return nullptr;
    }

    uint32_t version;
    stream >> version;
    if (version != BLIX_VERSION) {
        qWarning("Baloo/Index: unsupported version %u in %s",
                 version, qPrintable(path));
        return nullptr;
    }

    uint32_t wordCount;
    stream >> wordCount;

    BalooIndex *index = new BalooIndex;
    index->deletionCount = 0;
    index->totalEntries = 0;
    index->storagePath = QString::fromUtf8(storage_path);

    for (uint32_t w = 0; w < wordCount; ++w) {
        if (stream.atEnd())
            break;

        uint16_t wordLen;
        stream >> wordLen;

        QByteArray wordBytes(wordLen, '\0');
        stream.readRawData(wordBytes.data(), wordLen);
        QString word = QString::fromUtf8(wordBytes);

        uint32_t pathCount;
        stream >> pathCount;

        QVector<IndexEntry> entries;
        entries.reserve(static_cast<int>(pathCount));

        for (uint32_t p = 0; p < pathCount; ++p) {
            uint16_t pathLen;
            stream >> pathLen;

            QByteArray pathBytes(pathLen, '\0');
            stream.readRawData(pathBytes.data(), pathLen);

            int32_t relevance;
            stream >> relevance;

            IndexEntry entry;
            entry.path = QString::fromUtf8(pathBytes);
            entry.relevance = relevance;
            entries.append(entry);

            index->allPaths.insert(entry.path);
        }

        index->wordMap.insert(word, entries);
        index->totalEntries += entries.size();

        /* Rebuild trigram map */
        QSet<QString> trigrams = extractTrigrams(word);
        for (const QString &tri : trigrams)
            index->trigramMap[tri].insert(word);
    }

    qDebug("Baloo/Index: loaded %u words, %d entries from %s",
           wordCount, index->totalEntries, qPrintable(path));
    return index;
}

int baloo_index_save(BalooIndex *index, const char *storage_path)
{
    if (!index)
        return -1;

    QString dirPath = QString::fromUtf8(storage_path);
    QDir().mkpath(dirPath);

    QString path = dirPath + QStringLiteral("/baloo-index.blix");
    QFile file(path);
    if (!file.open(QIODevice::WriteOnly | QIODevice::Truncate)) {
        qWarning("Baloo/Index: cannot write %s", qPrintable(path));
        return -1;
    }

    QDataStream stream(&file);
    stream.setByteOrder(QDataStream::LittleEndian);

    /* Header */
    stream.writeRawData(BLIX_MAGIC, 4);
    stream << BLIX_VERSION;
    stream << static_cast<uint32_t>(index->wordMap.size());

    /* Word entries */
    for (auto it = index->wordMap.constBegin(); it != index->wordMap.constEnd(); ++it) {
        const QString &word = it.key();
        const QVector<IndexEntry> &entries = it.value();

        QByteArray wordUtf8 = word.toUtf8();
        stream << static_cast<uint16_t>(wordUtf8.size());
        stream.writeRawData(wordUtf8.constData(), wordUtf8.size());

        stream << static_cast<uint32_t>(entries.size());
        for (const IndexEntry &entry : entries) {
            QByteArray pathUtf8 = entry.path.toUtf8();
            stream << static_cast<uint16_t>(pathUtf8.size());
            stream.writeRawData(pathUtf8.constData(), pathUtf8.size());
            stream << static_cast<int32_t>(entry.relevance);
        }
    }

    qDebug("Baloo/Index: saved %d words to %s",
           index->wordMap.size(), qPrintable(path));
    return 0;
}

void baloo_index_destroy(BalooIndex *index)
{
    if (!index)
        return;

    qDebug("Baloo/Index: destroying index (%d words, %d entries)",
           index->wordMap.size(), index->totalEntries);
    delete index;
}

void baloo_index_add(BalooIndex *index, const char *word,
                     const char *file_path, int relevance)
{
    if (!index || !word || !file_path)
        return;

    QString w = QString::fromUtf8(word).toLower();
    QString p = QString::fromUtf8(file_path);

    if (w.isEmpty() || p.isEmpty())
        return;

    /* Add to word map */
    QVector<IndexEntry> &entries = index->wordMap[w];

    /* Avoid duplicate path entries for the same word -- update relevance */
    for (int i = 0; i < entries.size(); ++i) {
        if (entries[i].path == p) {
            if (entries[i].relevance < relevance)
                entries[i].relevance = relevance;
            return;
        }
    }

    entries.append({ p, relevance });
    index->totalEntries++;

    /* Add to trigram map */
    QSet<QString> trigrams = extractTrigrams(w);
    for (const QString &tri : trigrams)
        index->trigramMap[tri].insert(w);

    /* Track indexed paths */
    index->allPaths.insert(p);
}

void baloo_index_remove_file(BalooIndex *index, const char *file_path)
{
    if (!index || !file_path)
        return;

    QString p = QString::fromUtf8(file_path);

    /* Scan all word entries and remove those referencing the path */
    for (auto it = index->wordMap.begin(); it != index->wordMap.end(); ++it) {
        QVector<IndexEntry> &entries = it.value();
        int before = entries.size();

        entries.erase(
            std::remove_if(entries.begin(), entries.end(),
                           [&p](const IndexEntry &e) { return e.path == p; }),
            entries.end());

        int removed = before - entries.size();
        if (removed > 0)
            index->deletionCount += removed;
    }

    index->allPaths.remove(p);

    /* Compact if needed */
    compactIndex(index);
}

int baloo_index_search(BalooIndex *index, const char *query,
                       BalooIndexResult *results, int max)
{
    if (!index || !query || !results || max <= 0)
        return 0;

    QString q = QString::fromUtf8(query).toLower().trimmed();
    if (q.isEmpty())
        return 0;

    /* Tokenize query into words */
    QStringList queryWords = tokenize(q);
    if (queryWords.isEmpty()) {
        queryWords.append(q);
    }

    /* For each query word, find matching index words */
    QHash<QString, int> pathScores;  /* path -> aggregated relevance */

    for (const QString &qw : queryWords) {
        QSet<QString> matchingWords;

        /* 1. Direct word-prefix lookup */
        for (auto it = index->wordMap.constBegin();
             it != index->wordMap.constEnd(); ++it) {
            if (it.key().startsWith(qw))
                matchingWords.insert(it.key());
        }

        /* 2. Trigram-based substring search */
        QSet<QString> trigrams = extractTrigrams(qw);
        if (!trigrams.isEmpty()) {
            QSet<QString> candidates;
            bool first = true;

            for (const QString &tri : trigrams) {
                auto triIt = index->trigramMap.constFind(tri);
                if (triIt == index->trigramMap.constEnd()) {
                    candidates.clear();
                    break;
                }
                if (first) {
                    candidates = triIt.value();
                    first = false;
                } else {
                    candidates.intersect(triIt.value());
                }
            }

            /* Verify substring match (trigrams may have false positives) */
            for (const QString &candidate : candidates) {
                if (candidate.contains(qw))
                    matchingWords.insert(candidate);
            }
        }

        /* Aggregate scores from matching words */
        for (const QString &matchWord : matchingWords) {
            auto it = index->wordMap.constFind(matchWord);
            if (it == index->wordMap.constEnd())
                continue;

            for (const IndexEntry &entry : it.value()) {
                int bonus = 0;
                if (matchWord == qw)
                    bonus = 20;  /* Exact word match bonus */
                else if (matchWord.startsWith(qw))
                    bonus = 10;  /* Prefix match bonus */

                int score = entry.relevance + bonus;
                auto scoreIt = pathScores.find(entry.path);
                if (scoreIt != pathScores.end())
                    *scoreIt = qMax(*scoreIt, score);
                else
                    pathScores.insert(entry.path, score);
            }
        }
    }

    /* Sort by score descending */
    struct ScoredPath {
        QString path;
        int score;
    };
    QVector<ScoredPath> sorted;
    sorted.reserve(pathScores.size());

    for (auto it = pathScores.constBegin(); it != pathScores.constEnd(); ++it)
        sorted.append({ it.key(), it.value() });

    std::sort(sorted.begin(), sorted.end(),
              [](const ScoredPath &a, const ScoredPath &b) {
                  return a.score > b.score;
              });

    /* Fill results */
    int count = qMin(sorted.size(), max);
    for (int i = 0; i < count; ++i) {
        BalooIndexResult &r = results[i];
        memset(&r, 0, sizeof(r));

        QByteArray pathUtf8 = sorted[i].path.toUtf8();
        size_t len = static_cast<size_t>(pathUtf8.size());
        if (len >= sizeof(r.path))
            len = sizeof(r.path) - 1;
        memcpy(r.path, pathUtf8.constData(), len);
        r.path[len] = '\0';
        r.relevance = sorted[i].score;
    }

    return count;
}

uint64_t baloo_index_get_count(BalooIndex *index)
{
    if (!index)
        return 0;
    return static_cast<uint64_t>(index->allPaths.size());
}

} /* extern "C" */
