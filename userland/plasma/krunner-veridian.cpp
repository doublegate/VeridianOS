/*
 * VeridianOS -- krunner-veridian.cpp
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * KRunner search and launch framework implementation for VeridianOS.
 *
 * Built-in runners:
 *   - Applications: scans .desktop files, matches Name/GenericName/Keywords
 *   - Files:        delegates to Baloo if available, else substring filename search
 *   - Calculator:   recursive-descent integer arithmetic (+-*/^, parentheses)
 *   - Commands:     shell command execution (prefix ">" or absolute path)
 *   - Web search:   constructs DuckDuckGo/Google URLs for unmatched queries
 *   - Bookmarks:    scans browser bookmark files for URL matches
 *
 * D-Bus service: org.kde.krunner
 */

#include "krunner-veridian.h"

#include <QDebug>
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QTextStream>
#include <QDBusConnection>
#include <QDBusMessage>
#include <QProcess>
#include <QSettings>
#include <QUrl>
#include <QJsonDocument>
#include <QJsonObject>
#include <QJsonArray>

#include <string.h>
#include <stdlib.h>
#include <algorithm>

namespace KRunner {

/* ========================================================================= */
/* Runner configuration                                                      */
/* ========================================================================= */

static const int MAX_RUNNERS = 6;
static const int MAX_RESULTS_PER_RUNNER = 10;

struct RunnerState {
    const char *name;
    bool enabled;
};

static RunnerState s_runners[MAX_RUNNERS] = {
    { "applications", true  },
    { "files",        true  },
    { "calculator",   true  },
    { "commands",     true  },
    { "websearch",    true  },
    { "bookmarks",    true  },
};

static const char *s_runnerNames[MAX_RUNNERS + 1] = {
    "applications", "files", "calculator",
    "commands", "websearch", "bookmarks",
    nullptr
};

static bool s_initialized = false;

/* ========================================================================= */
/* Scoring helpers                                                           */
/* ========================================================================= */

static int scoreMatch(const QString &haystack, const QString &needle)
{
    if (haystack.isEmpty() || needle.isEmpty())
        return 0;

    QString h = haystack.toLower();
    QString n = needle.toLower();

    /* Exact match */
    if (h == n)
        return 100;

    /* Starts with query */
    if (h.startsWith(n))
        return 90;

    /* Word-prefix match: any word in haystack starts with needle */
    QStringList words = h.split(QRegularExpression(QStringLiteral("[\\s_\\-.]+")),
                                Qt::SkipEmptyParts);
    for (const QString &word : words) {
        if (word.startsWith(n))
            return 80;
    }

    /* Substring match */
    if (h.contains(n))
        return 60;

    /* Fuzzy: all chars of needle appear in order in haystack */
    int hi = 0;
    int ni = 0;
    while (hi < h.size() && ni < n.size()) {
        if (h[hi] == n[ni])
            ++ni;
        ++hi;
    }
    if (ni == n.size())
        return 40;

    return 0;
}

static void setMatchString(char *dst, size_t dstSize, const QString &src)
{
    QByteArray utf8 = src.toUtf8();
    size_t len = static_cast<size_t>(utf8.size());
    if (len >= dstSize)
        len = dstSize - 1;
    memcpy(dst, utf8.constData(), len);
    dst[len] = '\0';
}

/* ========================================================================= */
/* Desktop file parser (for applications runner)                             */
/* ========================================================================= */

struct DesktopEntry {
    QString name;
    QString genericName;
    QString comment;
    QString exec;
    QString icon;
    QString keywords;
    QString path;       /* .desktop file path */
    bool noDisplay;
    bool hidden;
};

static QVector<DesktopEntry> s_desktopEntries;
static bool s_desktopCacheDirty = true;

static DesktopEntry parseDesktopFile(const QString &path)
{
    DesktopEntry entry;
    entry.path = path;
    entry.noDisplay = false;
    entry.hidden = false;

    QFile file(path);
    if (!file.open(QIODevice::ReadOnly | QIODevice::Text))
        return entry;

    bool inDesktopEntry = false;
    QTextStream stream(&file);
    while (!stream.atEnd()) {
        QString line = stream.readLine().trimmed();

        if (line.startsWith(QLatin1Char('['))) {
            inDesktopEntry = (line == QStringLiteral("[Desktop Entry]"));
            continue;
        }

        if (!inDesktopEntry || line.isEmpty() || line.startsWith(QLatin1Char('#')))
            continue;

        int eqPos = line.indexOf(QLatin1Char('='));
        if (eqPos < 0)
            continue;

        QString key = line.left(eqPos).trimmed();
        QString value = line.mid(eqPos + 1).trimmed();

        if (key == QStringLiteral("Name"))
            entry.name = value;
        else if (key == QStringLiteral("GenericName"))
            entry.genericName = value;
        else if (key == QStringLiteral("Comment"))
            entry.comment = value;
        else if (key == QStringLiteral("Exec"))
            entry.exec = value;
        else if (key == QStringLiteral("Icon"))
            entry.icon = value;
        else if (key == QStringLiteral("Keywords"))
            entry.keywords = value;
        else if (key == QStringLiteral("NoDisplay"))
            entry.noDisplay = (value.toLower() == QStringLiteral("true"));
        else if (key == QStringLiteral("Hidden"))
            entry.hidden = (value.toLower() == QStringLiteral("true"));
    }

    return entry;
}

static void refreshDesktopCache()
{
    if (!s_desktopCacheDirty)
        return;

    s_desktopEntries.clear();

    QStringList searchPaths = {
        QStringLiteral("/usr/share/applications"),
        QStringLiteral("/usr/local/share/applications"),
        QDir::homePath() + QStringLiteral("/.local/share/applications"),
    };

    for (const QString &dirPath : searchPaths) {
        QDir dir(dirPath);
        if (!dir.exists())
            continue;

        QStringList filters = { QStringLiteral("*.desktop") };
        QFileInfoList entries = dir.entryInfoList(filters, QDir::Files);
        for (const QFileInfo &fi : entries) {
            DesktopEntry de = parseDesktopFile(fi.absoluteFilePath());
            if (de.name.isEmpty() || de.noDisplay || de.hidden)
                continue;
            s_desktopEntries.append(de);
        }
    }

    s_desktopCacheDirty = false;
    qDebug("KRunner/Applications: cached %d .desktop entries",
           s_desktopEntries.size());
}

/* ========================================================================= */
/* Applications runner                                                       */
/* ========================================================================= */

static int runApplications(const QString &query, KRunnerMatch *out, int maxOut)
{
    refreshDesktopCache();

    struct ScoredEntry {
        int score;
        int index;
    };
    QVector<ScoredEntry> scored;

    for (int i = 0; i < s_desktopEntries.size(); ++i) {
        const DesktopEntry &de = s_desktopEntries[i];

        int best = scoreMatch(de.name, query);
        best = qMax(best, scoreMatch(de.genericName, query));
        best = qMax(best, scoreMatch(de.keywords, query));

        if (best > 0)
            scored.append({ best, i });
    }

    /* Sort by score descending */
    std::sort(scored.begin(), scored.end(),
              [](const ScoredEntry &a, const ScoredEntry &b) {
                  return a.score > b.score;
              });

    int count = qMin(scored.size(), qMin(maxOut, MAX_RESULTS_PER_RUNNER));
    for (int i = 0; i < count; ++i) {
        const DesktopEntry &de = s_desktopEntries[scored[i].index];
        KRunnerMatch &m = out[i];
        memset(&m, 0, sizeof(m));

        setMatchString(m.text, sizeof(m.text), de.name);
        setMatchString(m.subtext, sizeof(m.subtext),
                       de.genericName.isEmpty() ? de.comment : de.genericName);
        setMatchString(m.icon_name, sizeof(m.icon_name), de.icon);
        m.relevance = scored[i].score;
        m.match_type = (scored[i].score >= 90) ? KRUNNER_EXACT_MATCH
                                                : KRUNNER_POSSIBLE_MATCH;
        setMatchString(m.data, sizeof(m.data),
                       QStringLiteral("app:%1").arg(de.path));
    }

    return count;
}

/* ========================================================================= */
/* Calculator runner -- recursive descent integer arithmetic                 */
/* ========================================================================= */

namespace Calc {

struct Token {
    enum Type { Number, Plus, Minus, Star, Slash, Caret, LParen, RParen, End, Error };
    Type type;
    int64_t value;  /* valid when type == Number */
};

struct Tokenizer {
    const char *pos;
    Token current;
};

static void advance(Tokenizer &t)
{
    /* Skip whitespace */
    while (*t.pos == ' ' || *t.pos == '\t')
        ++t.pos;

    if (*t.pos == '\0') {
        t.current = { Token::End, 0 };
        return;
    }

    char c = *t.pos;

    if (c >= '0' && c <= '9') {
        int64_t val = 0;
        while (*t.pos >= '0' && *t.pos <= '9') {
            /* Checked multiplication to avoid overflow */
            int64_t prev = val;
            val = val * 10 + (*t.pos - '0');
            if (val / 10 != prev) {
                t.current = { Token::Error, 0 };
                return;
            }
            ++t.pos;
        }
        t.current = { Token::Number, val };
        return;
    }

    ++t.pos;
    switch (c) {
    case '+': t.current = { Token::Plus,   0 }; return;
    case '-': t.current = { Token::Minus,  0 }; return;
    case '*': t.current = { Token::Star,   0 }; return;
    case '/': t.current = { Token::Slash,  0 }; return;
    case '^': t.current = { Token::Caret,  0 }; return;
    case '(': t.current = { Token::LParen, 0 }; return;
    case ')': t.current = { Token::RParen, 0 }; return;
    default:  t.current = { Token::Error,  0 }; return;
    }
}

/* Forward declarations for recursive descent */
static bool parseExpr(Tokenizer &t, int64_t &result);
static bool parseTerm(Tokenizer &t, int64_t &result);
static bool parsePower(Tokenizer &t, int64_t &result);
static bool parseFactor(Tokenizer &t, int64_t &result);

/* expr -> term (('+' | '-') term)* */
static bool parseExpr(Tokenizer &t, int64_t &result)
{
    if (!parseTerm(t, result))
        return false;

    while (t.current.type == Token::Plus || t.current.type == Token::Minus) {
        Token::Type op = t.current.type;
        advance(t);
        int64_t rhs;
        if (!parseTerm(t, rhs))
            return false;
        if (op == Token::Plus) {
            /* Checked addition */
            if ((rhs > 0 && result > INT64_MAX - rhs) ||
                (rhs < 0 && result < INT64_MIN - rhs))
                return false;
            result += rhs;
        } else {
            /* Checked subtraction */
            if ((rhs > 0 && result < INT64_MIN + rhs) ||
                (rhs < 0 && result > INT64_MAX + rhs))
                return false;
            result -= rhs;
        }
    }
    return true;
}

/* term -> power (('*' | '/') power)* */
static bool parseTerm(Tokenizer &t, int64_t &result)
{
    if (!parsePower(t, result))
        return false;

    while (t.current.type == Token::Star || t.current.type == Token::Slash) {
        Token::Type op = t.current.type;
        advance(t);
        int64_t rhs;
        if (!parsePower(t, rhs))
            return false;
        if (op == Token::Star) {
            /* Checked multiplication */
            if (result != 0 && rhs != 0) {
                if ((result > 0 && rhs > 0 && result > INT64_MAX / rhs) ||
                    (result < 0 && rhs < 0 && result < INT64_MAX / rhs) ||
                    (result > 0 && rhs < 0 && rhs < INT64_MIN / result) ||
                    (result < 0 && rhs > 0 && result < INT64_MIN / rhs))
                    return false;
            }
            result *= rhs;
        } else {
            if (rhs == 0)
                return false;  /* Division by zero */
            result /= rhs;
        }
    }
    return true;
}

/* power -> factor ('^' factor)* (right-associative) */
static bool parsePower(Tokenizer &t, int64_t &result)
{
    if (!parseFactor(t, result))
        return false;

    if (t.current.type == Token::Caret) {
        advance(t);
        int64_t exponent;
        if (!parsePower(t, exponent))  /* Right-associative recursion */
            return false;
        if (exponent < 0)
            return false;  /* No negative exponents in integer arithmetic */

        int64_t base = result;
        result = 1;
        for (int64_t i = 0; i < exponent; ++i) {
            /* Checked multiplication */
            if (base != 0 && result != 0) {
                int64_t prev = result;
                result *= base;
                if (result / base != prev)
                    return false;  /* Overflow */
            } else {
                result *= base;
            }
        }
    }
    return true;
}

/* factor -> NUMBER | '(' expr ')' | '-' factor */
static bool parseFactor(Tokenizer &t, int64_t &result)
{
    if (t.current.type == Token::Number) {
        result = t.current.value;
        advance(t);
        return true;
    }

    if (t.current.type == Token::Minus) {
        advance(t);
        if (!parseFactor(t, result))
            return false;
        if (result == INT64_MIN)
            return false;  /* Cannot negate INT64_MIN */
        result = -result;
        return true;
    }

    if (t.current.type == Token::LParen) {
        advance(t);
        if (!parseExpr(t, result))
            return false;
        if (t.current.type != Token::RParen)
            return false;  /* Missing closing paren */
        advance(t);
        return true;
    }

    return false;  /* Unexpected token */
}

} /* namespace Calc */

static int runCalculator(const QString &query, KRunnerMatch *out, int maxOut)
{
    if (maxOut < 1)
        return 0;

    /* Must contain at least one operator to be considered an expression */
    bool hasOperator = false;
    for (QChar c : query) {
        if (c == QLatin1Char('+') || c == QLatin1Char('-') ||
            c == QLatin1Char('*') || c == QLatin1Char('/') ||
            c == QLatin1Char('^') || c == QLatin1Char('(')) {
            hasOperator = true;
            break;
        }
    }
    if (!hasOperator)
        return 0;

    QByteArray utf8 = query.toUtf8();
    Calc::Tokenizer tokenizer;
    tokenizer.pos = utf8.constData();
    Calc::advance(tokenizer);

    int64_t result;
    if (!Calc::parseExpr(tokenizer, result))
        return 0;
    if (tokenizer.current.type != Calc::Token::End)
        return 0;  /* Trailing garbage */

    KRunnerMatch &m = out[0];
    memset(&m, 0, sizeof(m));

    QString resultStr = QStringLiteral("%1 = %2").arg(query).arg(result);
    setMatchString(m.text, sizeof(m.text), resultStr);
    setMatchString(m.subtext, sizeof(m.subtext),
                   QStringLiteral("Calculator result"));
    setMatchString(m.icon_name, sizeof(m.icon_name),
                   QStringLiteral("accessories-calculator"));
    m.relevance = 100;
    m.match_type = KRUNNER_INFORMATIONAL_MATCH;
    setMatchString(m.data, sizeof(m.data),
                   QStringLiteral("calc:%1").arg(result));

    return 1;
}

/* ========================================================================= */
/* File search runner                                                        */
/* ========================================================================= */

static int runFiles(const QString &query, KRunnerMatch *out, int maxOut)
{
    if (query.length() < 3 || maxOut < 1)
        return 0;

    /* Try Baloo D-Bus interface first */
    QDBusMessage balooMsg = QDBusMessage::createMethodCall(
        QStringLiteral("org.kde.baloo"),
        QStringLiteral("/"),
        QStringLiteral("org.kde.baloo.main"),
        QStringLiteral("search"));
    balooMsg << query << 10;  /* query, maxResults */

    QDBusMessage reply = QDBusConnection::sessionBus().call(balooMsg, QDBus::Block, 500);
    if (reply.type() == QDBusMessage::ReplyMessage && !reply.arguments().isEmpty()) {
        QStringList paths = reply.arguments().first().toStringList();
        int count = qMin(paths.size(), qMin(maxOut, MAX_RESULTS_PER_RUNNER));
        for (int i = 0; i < count; ++i) {
            QFileInfo fi(paths[i]);
            KRunnerMatch &m = out[i];
            memset(&m, 0, sizeof(m));

            setMatchString(m.text, sizeof(m.text), fi.fileName());
            setMatchString(m.subtext, sizeof(m.subtext), fi.absolutePath());
            setMatchString(m.icon_name, sizeof(m.icon_name),
                           fi.isDir() ? QStringLiteral("folder")
                                      : QStringLiteral("text-x-generic"));
            m.relevance = 80 - i * 5;
            m.match_type = KRUNNER_POSSIBLE_MATCH;
            setMatchString(m.data, sizeof(m.data),
                           QStringLiteral("file:%1").arg(fi.absoluteFilePath()));
        }
        return count;
    }

    /* Fallback: simple filename search in home directory */
    QDir homeDir(QDir::homePath());
    QStringList results;

    QStringList nameFilters = { QStringLiteral("*%1*").arg(query) };
    QFileInfoList entries = homeDir.entryInfoList(nameFilters,
                                                   QDir::Files | QDir::Dirs |
                                                   QDir::NoDotAndDotDot);

    int count = qMin(entries.size(), qMin(maxOut, MAX_RESULTS_PER_RUNNER));
    for (int i = 0; i < count; ++i) {
        const QFileInfo &fi = entries[i];
        KRunnerMatch &m = out[i];
        memset(&m, 0, sizeof(m));

        setMatchString(m.text, sizeof(m.text), fi.fileName());
        setMatchString(m.subtext, sizeof(m.subtext), fi.absolutePath());
        setMatchString(m.icon_name, sizeof(m.icon_name),
                       fi.isDir() ? QStringLiteral("folder")
                                  : QStringLiteral("text-x-generic"));
        m.relevance = scoreMatch(fi.fileName(), query);
        m.match_type = KRUNNER_POSSIBLE_MATCH;
        setMatchString(m.data, sizeof(m.data),
                       QStringLiteral("file:%1").arg(fi.absoluteFilePath()));
    }

    return count;
}

/* ========================================================================= */
/* Command runner                                                            */
/* ========================================================================= */

static int runCommands(const QString &query, KRunnerMatch *out, int maxOut)
{
    if (maxOut < 1)
        return 0;

    QString cmd;
    if (query.startsWith(QLatin1Char('>'))) {
        cmd = query.mid(1).trimmed();
    } else if (query.startsWith(QLatin1Char('/'))) {
        /* Absolute path -- offer to run it */
        if (QFileInfo(query).isExecutable())
            cmd = query;
        else
            return 0;
    } else {
        return 0;
    }

    if (cmd.isEmpty())
        return 0;

    KRunnerMatch &m = out[0];
    memset(&m, 0, sizeof(m));

    setMatchString(m.text, sizeof(m.text),
                   QStringLiteral("Run: %1").arg(cmd));
    setMatchString(m.subtext, sizeof(m.subtext),
                   QStringLiteral("Execute command in shell"));
    setMatchString(m.icon_name, sizeof(m.icon_name),
                   QStringLiteral("system-run"));
    m.relevance = 90;
    m.match_type = KRUNNER_HELPER_MATCH;
    setMatchString(m.data, sizeof(m.data),
                   QStringLiteral("cmd:%1").arg(cmd));

    return 1;
}

/* ========================================================================= */
/* Web search runner                                                         */
/* ========================================================================= */

static int runWebSearch(const QString &query, KRunnerMatch *out, int maxOut)
{
    if (maxOut < 1 || query.length() < 2)
        return 0;

    int count = 0;
    QString encoded = QUrl::toPercentEncoding(query);

    /* DuckDuckGo */
    if (count < maxOut) {
        KRunnerMatch &m = out[count];
        memset(&m, 0, sizeof(m));
        setMatchString(m.text, sizeof(m.text),
                       QStringLiteral("Search DuckDuckGo for \"%1\"").arg(query));
        setMatchString(m.subtext, sizeof(m.subtext),
                       QStringLiteral("https://duckduckgo.com"));
        setMatchString(m.icon_name, sizeof(m.icon_name),
                       QStringLiteral("internet-web-browser"));
        m.relevance = 30;
        m.match_type = KRUNNER_HELPER_MATCH;
        setMatchString(m.data, sizeof(m.data),
                       QStringLiteral("url:https://duckduckgo.com/?q=%1").arg(encoded));
        ++count;
    }

    /* Google */
    if (count < maxOut) {
        KRunnerMatch &m = out[count];
        memset(&m, 0, sizeof(m));
        setMatchString(m.text, sizeof(m.text),
                       QStringLiteral("Search Google for \"%1\"").arg(query));
        setMatchString(m.subtext, sizeof(m.subtext),
                       QStringLiteral("https://www.google.com"));
        setMatchString(m.icon_name, sizeof(m.icon_name),
                       QStringLiteral("internet-web-browser"));
        m.relevance = 25;
        m.match_type = KRUNNER_HELPER_MATCH;
        setMatchString(m.data, sizeof(m.data),
                       QStringLiteral("url:https://www.google.com/search?q=%1").arg(encoded));
        ++count;
    }

    return count;
}

/* ========================================================================= */
/* Bookmark runner                                                           */
/* ========================================================================= */

static int runBookmarks(const QString &query, KRunnerMatch *out, int maxOut)
{
    if (maxOut < 1 || query.length() < 2)
        return 0;

    struct BookmarkEntry {
        QString title;
        QString url;
        int score;
    };
    QVector<BookmarkEntry> matches;

    /* Scan Konqueror/Falkon-style bookmarks.xml */
    QString konqPath = QDir::homePath() +
                       QStringLiteral("/.local/share/konqueror/bookmarks.xml");
    QFile konqFile(konqPath);
    if (konqFile.open(QIODevice::ReadOnly | QIODevice::Text)) {
        QTextStream stream(&konqFile);
        QString content = stream.readAll();

        /* Simple XML parsing: look for <bookmark href="..." title="..."> */
        int pos = 0;
        while ((pos = content.indexOf(QStringLiteral("<bookmark"), pos)) >= 0) {
            int endTag = content.indexOf(QLatin1Char('>'), pos);
            if (endTag < 0)
                break;
            QString tag = content.mid(pos, endTag - pos);

            /* Extract href */
            int hrefPos = tag.indexOf(QStringLiteral("href=\""));
            int titlePos = tag.indexOf(QStringLiteral("title=\""));
            if (hrefPos >= 0) {
                hrefPos += 6;
                int hrefEnd = tag.indexOf(QLatin1Char('"'), hrefPos);
                QString url = tag.mid(hrefPos, hrefEnd - hrefPos);

                QString title;
                if (titlePos >= 0) {
                    titlePos += 7;
                    int titleEnd = tag.indexOf(QLatin1Char('"'), titlePos);
                    title = tag.mid(titlePos, titleEnd - titlePos);
                }

                int s = qMax(scoreMatch(title, query), scoreMatch(url, query));
                if (s > 0)
                    matches.append({ title, url, s });
            }
            pos = endTag;
        }
    }

    /* Scan Chromium JSON bookmarks */
    QString chromiumPath = QDir::homePath() +
                           QStringLiteral("/.config/chromium/Default/Bookmarks");
    QFile chromiumFile(chromiumPath);
    if (chromiumFile.open(QIODevice::ReadOnly)) {
        QJsonDocument doc = QJsonDocument::fromJson(chromiumFile.readAll());
        QJsonObject roots = doc.object().value(QStringLiteral("roots")).toObject();

        /* Recursively scan bookmark bar and other folders */
        std::function<void(const QJsonObject &)> scanFolder;
        scanFolder = [&](const QJsonObject &obj) {
            QString type = obj.value(QStringLiteral("type")).toString();
            if (type == QStringLiteral("url")) {
                QString name = obj.value(QStringLiteral("name")).toString();
                QString url = obj.value(QStringLiteral("url")).toString();
                int s = qMax(scoreMatch(name, query), scoreMatch(url, query));
                if (s > 0)
                    matches.append({ name, url, s });
            } else if (type == QStringLiteral("folder")) {
                QJsonArray children = obj.value(QStringLiteral("children")).toArray();
                for (const QJsonValue &child : children)
                    scanFolder(child.toObject());
            }
        };

        for (const QString &key : roots.keys())
            scanFolder(roots.value(key).toObject());
    }

    /* Sort by score descending */
    std::sort(matches.begin(), matches.end(),
              [](const BookmarkEntry &a, const BookmarkEntry &b) {
                  return a.score > b.score;
              });

    int count = qMin(matches.size(), qMin(maxOut, MAX_RESULTS_PER_RUNNER));
    for (int i = 0; i < count; ++i) {
        KRunnerMatch &m = out[i];
        memset(&m, 0, sizeof(m));

        setMatchString(m.text, sizeof(m.text), matches[i].title);
        setMatchString(m.subtext, sizeof(m.subtext), matches[i].url);
        setMatchString(m.icon_name, sizeof(m.icon_name),
                       QStringLiteral("bookmarks"));
        m.relevance = matches[i].score;
        m.match_type = KRUNNER_POSSIBLE_MATCH;
        setMatchString(m.data, sizeof(m.data),
                       QStringLiteral("url:%1").arg(matches[i].url));
    }

    return count;
}

/* ========================================================================= */
/* Match execution                                                           */
/* ========================================================================= */

static void executeMatch(const QString &data)
{
    if (data.startsWith(QStringLiteral("app:"))) {
        /* Launch .desktop file */
        QString desktopPath = data.mid(4);
        DesktopEntry entry = parseDesktopFile(desktopPath);
        if (!entry.exec.isEmpty()) {
            /* Remove %f, %F, %u, %U, %d, %D, %n, %N, %i, %c, %k field codes */
            QString cmd = entry.exec;
            cmd.remove(QRegularExpression(QStringLiteral("%[fFuUdDnNickK]")));
            cmd = cmd.trimmed();
            qDebug("KRunner: launching app: %s", qPrintable(cmd));
            QProcess::startDetached(QStringLiteral("/bin/sh"),
                                    QStringList{ QStringLiteral("-c"), cmd });
        }
    } else if (data.startsWith(QStringLiteral("file:"))) {
        /* Open file with default handler via xdg-open */
        QString path = data.mid(5);
        qDebug("KRunner: opening file: %s", qPrintable(path));
        QProcess::startDetached(QStringLiteral("xdg-open"),
                                QStringList{ path });
    } else if (data.startsWith(QStringLiteral("cmd:"))) {
        /* Execute shell command */
        QString cmd = data.mid(4);
        qDebug("KRunner: executing command: %s", qPrintable(cmd));
        QProcess::startDetached(QStringLiteral("/bin/sh"),
                                QStringList{ QStringLiteral("-c"), cmd });
    } else if (data.startsWith(QStringLiteral("url:"))) {
        /* Open URL in browser */
        QString url = data.mid(4);
        qDebug("KRunner: opening URL: %s", qPrintable(url));
        QProcess::startDetached(QStringLiteral("xdg-open"),
                                QStringList{ url });
    } else if (data.startsWith(QStringLiteral("calc:"))) {
        /* Calculator result -- copy to clipboard via D-Bus */
        QString result = data.mid(5);
        QDBusMessage msg = QDBusMessage::createMethodCall(
            QStringLiteral("org.kde.klipper"),
            QStringLiteral("/klipper"),
            QStringLiteral("org.kde.klipper.klipper"),
            QStringLiteral("setClipboardContents"));
        msg << result;
        QDBusConnection::sessionBus().send(msg);
        qDebug("KRunner: copied calculator result to clipboard: %s",
               qPrintable(result));
    }
}

/* ========================================================================= */
/* D-Bus registration                                                        */
/* ========================================================================= */

static bool registerDBus()
{
    QDBusConnection bus = QDBusConnection::sessionBus();
    if (!bus.registerService(QStringLiteral("org.kde.krunner"))) {
        qWarning("KRunner: failed to register D-Bus service: %s",
                 qPrintable(bus.lastError().message()));
        return false;
    }
    qDebug("KRunner: D-Bus service registered at org.kde.krunner");
    return true;
}

} /* namespace KRunner */

/* ========================================================================= */
/* C API implementation                                                      */
/* ========================================================================= */

extern "C" {

int krunner_init(void)
{
    if (KRunner::s_initialized)
        return 0;

    KRunner::registerDBus();
    KRunner::s_desktopCacheDirty = true;
    KRunner::s_initialized = true;

    qDebug("KRunner: initialized with %d runners", KRunner::MAX_RUNNERS);
    return 0;
}

void krunner_destroy(void)
{
    if (!KRunner::s_initialized)
        return;

    KRunner::s_desktopEntries.clear();
    KRunner::s_initialized = false;

    QDBusConnection::sessionBus().unregisterService(
        QStringLiteral("org.kde.krunner"));

    qDebug("KRunner: destroyed");
}

int krunner_query(const char *query_string,
                  KRunnerMatch *matches_out,
                  int max_matches)
{
    if (!KRunner::s_initialized || !query_string || !matches_out || max_matches <= 0)
        return -1;

    QString query = QString::fromUtf8(query_string).trimmed();
    if (query.isEmpty())
        return 0;

    /* Collect results from all enabled runners */
    QVector<KRunnerMatch> allMatches;
    KRunnerMatch tmpBuf[KRunner::MAX_RESULTS_PER_RUNNER];

    for (int r = 0; r < KRunner::MAX_RUNNERS; ++r) {
        if (!KRunner::s_runners[r].enabled)
            continue;

        int count = 0;
        const char *name = KRunner::s_runners[r].name;

        if (strcmp(name, "applications") == 0)
            count = KRunner::runApplications(query, tmpBuf, KRunner::MAX_RESULTS_PER_RUNNER);
        else if (strcmp(name, "files") == 0)
            count = KRunner::runFiles(query, tmpBuf, KRunner::MAX_RESULTS_PER_RUNNER);
        else if (strcmp(name, "calculator") == 0)
            count = KRunner::runCalculator(query, tmpBuf, KRunner::MAX_RESULTS_PER_RUNNER);
        else if (strcmp(name, "commands") == 0)
            count = KRunner::runCommands(query, tmpBuf, KRunner::MAX_RESULTS_PER_RUNNER);
        else if (strcmp(name, "websearch") == 0)
            count = KRunner::runWebSearch(query, tmpBuf, KRunner::MAX_RESULTS_PER_RUNNER);
        else if (strcmp(name, "bookmarks") == 0)
            count = KRunner::runBookmarks(query, tmpBuf, KRunner::MAX_RESULTS_PER_RUNNER);

        for (int i = 0; i < count; ++i)
            allMatches.append(tmpBuf[i]);
    }

    /* Sort all results by relevance descending */
    std::sort(allMatches.begin(), allMatches.end(),
              [](const KRunnerMatch &a, const KRunnerMatch &b) {
                  return a.relevance > b.relevance;
              });

    int count = qMin(allMatches.size(), max_matches);
    for (int i = 0; i < count; ++i)
        matches_out[i] = allMatches[i];

    return count;
}

void krunner_run(const char *match_data)
{
    if (!match_data)
        return;

    KRunner::executeMatch(QString::fromUtf8(match_data));
}

const char **krunner_get_runners(void)
{
    return KRunner::s_runnerNames;
}

void krunner_enable_runner(const char *name)
{
    if (!name)
        return;
    for (int i = 0; i < KRunner::MAX_RUNNERS; ++i) {
        if (strcmp(KRunner::s_runners[i].name, name) == 0) {
            KRunner::s_runners[i].enabled = true;
            qDebug("KRunner: enabled runner '%s'", name);
            return;
        }
    }
    qWarning("KRunner: unknown runner '%s'", name);
}

void krunner_disable_runner(const char *name)
{
    if (!name)
        return;
    for (int i = 0; i < KRunner::MAX_RUNNERS; ++i) {
        if (strcmp(KRunner::s_runners[i].name, name) == 0) {
            KRunner::s_runners[i].enabled = false;
            qDebug("KRunner: disabled runner '%s'", name);
            return;
        }
    }
    qWarning("KRunner: unknown runner '%s'", name);
}

} /* extern "C" */
