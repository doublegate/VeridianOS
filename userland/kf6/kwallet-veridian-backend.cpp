/*
 * VeridianOS -- kwallet-veridian-backend.cpp
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * KWallet backend implementation for VeridianOS.  Simple file-based
 * credential storage with XOR obfuscation (initial implementation).
 *
 * TODO: Replace XOR with AES-256-GCM for production use.
 */

#include "kwallet-veridian-backend.h"

#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QDataStream>
#include <QStandardPaths>
#include <QCryptographicHash>
#include <QDebug>

namespace KWallet {

/* File format magic and version */
static const char WALLET_MAGIC[4] = { 'V', 'K', 'W', 'L' };
static const quint32 WALLET_VERSION = 1;

/* Default folder name */
static const QString DEFAULT_FOLDER = QStringLiteral("Passwords");

/* ========================================================================= */
/* Construction / destruction                                                */
/* ========================================================================= */

VeridianWalletBackend::VeridianWalletBackend(QObject *parent)
    : QObject(parent)
    , m_currentFolder(DEFAULT_FOLDER)
    , m_open(false)
    , m_dirty(false)
{
}

VeridianWalletBackend::~VeridianWalletBackend()
{
    if (m_open)
        closeWallet();
}

/* ========================================================================= */
/* Wallet lifecycle                                                          */
/* ========================================================================= */

int VeridianWalletBackend::openWallet(const QString &name,
                                       const QString &password)
{
    if (m_open)
        closeWallet();

    m_walletName = name;
    m_encryptionKey = deriveKey(password);
    m_currentFolder = DEFAULT_FOLDER;
    m_dirty = false;

    /* Try to load existing wallet file */
    QString path = walletFilePath(name);
    if (QFileInfo::exists(path)) {
        if (!loadFromFile(name, m_encryptionKey)) {
            qWarning("KWallet: Failed to load wallet '%s'",
                     qPrintable(name));
            m_encryptionKey.clear();
            return -1;
        }
    } else {
        /* Create new wallet with default folder */
        WalletFolder defaultFolder;
        defaultFolder.name = DEFAULT_FOLDER;
        m_folders.insert(DEFAULT_FOLDER, defaultFolder);
        m_dirty = true;
    }

    m_open = true;
    Q_EMIT walletOpened(name);
    return 0;
}

int VeridianWalletBackend::closeWallet()
{
    if (!m_open)
        return 0;

    if (m_dirty) {
        if (!saveToFile()) {
            qWarning("KWallet: Failed to save wallet '%s'",
                     qPrintable(m_walletName));
            return -1;
        }
    }

    m_walletName.clear();
    m_encryptionKey.clear();
    m_folders.clear();
    m_currentFolder = DEFAULT_FOLDER;
    m_open = false;
    m_dirty = false;

    Q_EMIT walletClosed();
    return 0;
}

bool VeridianWalletBackend::isOpen() const
{
    return m_open;
}

QString VeridianWalletBackend::currentWallet() const
{
    return m_walletName;
}

QStringList VeridianWalletBackend::wallets() const
{
    QDir dir(walletDirectory());
    QStringList result;
    const QStringList entries = dir.entryList(
        QStringList(QStringLiteral("*.kwl")), QDir::Files);
    for (const QString &entry : entries) {
        result << entry.left(entry.length() - 4); /* strip .kwl */
    }
    return result;
}

int VeridianWalletBackend::deleteWallet(const QString &name)
{
    if (m_open && m_walletName == name)
        closeWallet();

    QString path = walletFilePath(name);
    if (QFile::remove(path))
        return 0;
    return -1;
}

/* ========================================================================= */
/* Folder operations                                                         */
/* ========================================================================= */

QStringList VeridianWalletBackend::folderList() const
{
    return QStringList(m_folders.keys());
}

bool VeridianWalletBackend::hasFolder(const QString &folder) const
{
    return m_folders.contains(folder);
}

bool VeridianWalletBackend::createFolder(const QString &folder)
{
    if (m_folders.contains(folder))
        return true;

    WalletFolder f;
    f.name = folder;
    m_folders.insert(folder, f);
    m_dirty = true;
    return true;
}

bool VeridianWalletBackend::removeFolder(const QString &folder)
{
    if (!m_folders.contains(folder))
        return false;

    m_folders.remove(folder);
    if (m_currentFolder == folder)
        m_currentFolder = DEFAULT_FOLDER;
    m_dirty = true;
    return true;
}

void VeridianWalletBackend::setFolder(const QString &folder)
{
    if (!m_folders.contains(folder))
        createFolder(folder);
    m_currentFolder = folder;
}

QString VeridianWalletBackend::currentFolder() const
{
    return m_currentFolder;
}

/* ========================================================================= */
/* Entry operations                                                          */
/* ========================================================================= */

int VeridianWalletBackend::readPassword(const QString &key,
                                         QString &value) const
{
    if (!m_open || !m_folders.contains(m_currentFolder))
        return -1;

    const WalletFolder &folder = m_folders[m_currentFolder];
    auto it = folder.entries.constFind(key);
    if (it == folder.entries.constEnd())
        return -1;

    value = QString::fromUtf8(it->value);
    return 0;
}

int VeridianWalletBackend::writePassword(const QString &key,
                                          const QString &value)
{
    if (!m_open)
        return -1;

    if (!m_folders.contains(m_currentFolder))
        createFolder(m_currentFolder);

    WalletEntry entry;
    entry.key = key;
    entry.value = value.toUtf8();
    entry.type = Password;

    m_folders[m_currentFolder].entries.insert(key, entry);
    m_dirty = true;

    Q_EMIT folderUpdated(m_currentFolder);
    return 0;
}

int VeridianWalletBackend::readEntry(const QString &key,
                                      QByteArray &value) const
{
    if (!m_open || !m_folders.contains(m_currentFolder))
        return -1;

    const WalletFolder &folder = m_folders[m_currentFolder];
    auto it = folder.entries.constFind(key);
    if (it == folder.entries.constEnd())
        return -1;

    value = it->value;
    return 0;
}

int VeridianWalletBackend::writeEntry(const QString &key,
                                       const QByteArray &value)
{
    if (!m_open)
        return -1;

    if (!m_folders.contains(m_currentFolder))
        createFolder(m_currentFolder);

    WalletEntry entry;
    entry.key = key;
    entry.value = value;
    entry.type = Stream;

    m_folders[m_currentFolder].entries.insert(key, entry);
    m_dirty = true;

    Q_EMIT folderUpdated(m_currentFolder);
    return 0;
}

int VeridianWalletBackend::readMap(const QString &key,
                                    QMap<QString, QString> &value) const
{
    if (!m_open || !m_folders.contains(m_currentFolder))
        return -1;

    const WalletFolder &folder = m_folders[m_currentFolder];
    auto it = folder.entries.constFind(key);
    if (it == folder.entries.constEnd() || it->type != Map)
        return -1;

    /* Deserialize QMap from QByteArray */
    QDataStream stream(it->value);
    stream >> value;
    return 0;
}

int VeridianWalletBackend::writeMap(const QString &key,
                                     const QMap<QString, QString> &value)
{
    if (!m_open)
        return -1;

    if (!m_folders.contains(m_currentFolder))
        createFolder(m_currentFolder);

    /* Serialize QMap to QByteArray */
    QByteArray data;
    QDataStream stream(&data, QIODevice::WriteOnly);
    stream << value;

    WalletEntry entry;
    entry.key = key;
    entry.value = data;
    entry.type = Map;

    m_folders[m_currentFolder].entries.insert(key, entry);
    m_dirty = true;

    Q_EMIT folderUpdated(m_currentFolder);
    return 0;
}

bool VeridianWalletBackend::hasEntry(const QString &key) const
{
    if (!m_open || !m_folders.contains(m_currentFolder))
        return false;
    return m_folders[m_currentFolder].entries.contains(key);
}

int VeridianWalletBackend::removeEntry(const QString &key)
{
    if (!m_open || !m_folders.contains(m_currentFolder))
        return -1;

    if (m_folders[m_currentFolder].entries.remove(key) == 0)
        return -1;

    m_dirty = true;
    Q_EMIT folderUpdated(m_currentFolder);
    return 0;
}

int VeridianWalletBackend::renameEntry(const QString &oldKey,
                                        const QString &newKey)
{
    if (!m_open || !m_folders.contains(m_currentFolder))
        return -1;

    WalletFolder &folder = m_folders[m_currentFolder];
    auto it = folder.entries.find(oldKey);
    if (it == folder.entries.end())
        return -1;

    WalletEntry entry = it.value();
    entry.key = newKey;
    folder.entries.erase(it);
    folder.entries.insert(newKey, entry);
    m_dirty = true;

    Q_EMIT folderUpdated(m_currentFolder);
    return 0;
}

QStringList VeridianWalletBackend::entryList() const
{
    if (!m_open || !m_folders.contains(m_currentFolder))
        return QStringList();

    return QStringList(m_folders[m_currentFolder].entries.keys());
}

EntryType VeridianWalletBackend::entryType(const QString &key) const
{
    if (!m_open || !m_folders.contains(m_currentFolder))
        return Unknown;

    auto it = m_folders[m_currentFolder].entries.constFind(key);
    if (it == m_folders[m_currentFolder].entries.constEnd())
        return Unknown;

    return it->type;
}

/* ========================================================================= */
/* Encryption                                                                */
/* ========================================================================= */

QByteArray VeridianWalletBackend::deriveKey(const QString &password) const
{
    /* Derive a 256-bit key from the password using SHA-256.
     * TODO: Use PBKDF2 or Argon2 for production. */
    return QCryptographicHash::hash(password.toUtf8(),
                                    QCryptographicHash::Sha256);
}

QByteArray VeridianWalletBackend::encrypt(const QByteArray &data,
                                           const QByteArray &key) const
{
    /* Simple XOR encryption (placeholder -- replace with AES-256-GCM).
     * This provides basic obfuscation but NOT real security. */
    QByteArray result = data;
    for (int i = 0; i < result.size(); ++i) {
        result[i] = result[i] ^ key[i % key.size()];
    }
    return result;
}

QByteArray VeridianWalletBackend::decrypt(const QByteArray &data,
                                           const QByteArray &key) const
{
    /* XOR is its own inverse */
    return encrypt(data, key);
}

/* ========================================================================= */
/* Serialization                                                             */
/* ========================================================================= */

QByteArray VeridianWalletBackend::serialize() const
{
    QByteArray data;
    QDataStream stream(&data, QIODevice::WriteOnly);
    stream.setByteOrder(QDataStream::LittleEndian);

    /* Number of folders */
    stream << static_cast<quint32>(m_folders.size());

    for (auto folderIt = m_folders.constBegin();
         folderIt != m_folders.constEnd(); ++folderIt) {
        const WalletFolder &folder = folderIt.value();

        /* Folder name */
        QByteArray nameUtf8 = folder.name.toUtf8();
        stream << static_cast<quint32>(nameUtf8.size());
        stream.writeRawData(nameUtf8.constData(), nameUtf8.size());

        /* Number of entries */
        stream << static_cast<quint32>(folder.entries.size());

        for (auto entryIt = folder.entries.constBegin();
             entryIt != folder.entries.constEnd(); ++entryIt) {
            const WalletEntry &entry = entryIt.value();

            /* Key */
            QByteArray keyUtf8 = entry.key.toUtf8();
            stream << static_cast<quint32>(keyUtf8.size());
            stream.writeRawData(keyUtf8.constData(), keyUtf8.size());

            /* Type */
            stream << static_cast<quint32>(entry.type);

            /* Value */
            stream << static_cast<quint32>(entry.value.size());
            stream.writeRawData(entry.value.constData(), entry.value.size());
        }
    }

    return data;
}

bool VeridianWalletBackend::deserialize(const QByteArray &data)
{
    QDataStream stream(data);
    stream.setByteOrder(QDataStream::LittleEndian);

    m_folders.clear();

    quint32 numFolders;
    stream >> numFolders;
    if (stream.status() != QDataStream::Ok || numFolders > 1000)
        return false;

    for (quint32 f = 0; f < numFolders; ++f) {
        WalletFolder folder;

        /* Folder name */
        quint32 nameLen;
        stream >> nameLen;
        if (stream.status() != QDataStream::Ok || nameLen > 10000)
            return false;
        QByteArray nameData(static_cast<int>(nameLen), '\0');
        stream.readRawData(nameData.data(), static_cast<int>(nameLen));
        folder.name = QString::fromUtf8(nameData);

        /* Number of entries */
        quint32 numEntries;
        stream >> numEntries;
        if (stream.status() != QDataStream::Ok || numEntries > 100000)
            return false;

        for (quint32 e = 0; e < numEntries; ++e) {
            WalletEntry entry;

            /* Key */
            quint32 keyLen;
            stream >> keyLen;
            if (stream.status() != QDataStream::Ok || keyLen > 10000)
                return false;
            QByteArray keyData(static_cast<int>(keyLen), '\0');
            stream.readRawData(keyData.data(), static_cast<int>(keyLen));
            entry.key = QString::fromUtf8(keyData);

            /* Type */
            quint32 type;
            stream >> type;
            entry.type = static_cast<EntryType>(type);

            /* Value */
            quint32 valueLen;
            stream >> valueLen;
            if (stream.status() != QDataStream::Ok || valueLen > 10000000)
                return false;
            entry.value.resize(static_cast<int>(valueLen));
            stream.readRawData(entry.value.data(), static_cast<int>(valueLen));

            folder.entries.insert(entry.key, entry);
        }

        m_folders.insert(folder.name, folder);
    }

    return true;
}

/* ========================================================================= */
/* File I/O                                                                  */
/* ========================================================================= */

QString VeridianWalletBackend::walletFilePath(const QString &name) const
{
    return walletDirectory() + QLatin1Char('/') + name +
           QStringLiteral(".kwl");
}

QString VeridianWalletBackend::walletDirectory() const
{
    QString dataHome = QStandardPaths::writableLocation(
        QStandardPaths::GenericDataLocation);
    if (dataHome.isEmpty())
        dataHome = QDir::homePath() + QStringLiteral("/.local/share");
    return dataHome + QStringLiteral("/kwallet");
}

bool VeridianWalletBackend::saveToFile()
{
    /* Ensure directory exists */
    QDir dir;
    dir.mkpath(walletDirectory());

    /* Serialize wallet data */
    QByteArray plaintext = serialize();
    QByteArray ciphertext = encrypt(plaintext, m_encryptionKey);

    /* Write file */
    QFile file(walletFilePath(m_walletName));
    if (!file.open(QIODevice::WriteOnly)) {
        qWarning("KWallet: Cannot open '%s' for writing",
                 qPrintable(file.fileName()));
        return false;
    }

    /* Write header */
    file.write(WALLET_MAGIC, 4);
    quint32 version = WALLET_VERSION;
    file.write(reinterpret_cast<const char *>(&version), 4);

    /* Write encrypted payload */
    file.write(ciphertext);
    file.close();

    /* Set restrictive permissions (owner-only) */
    file.setPermissions(QFile::ReadOwner | QFile::WriteOwner);

    m_dirty = false;
    return true;
}

bool VeridianWalletBackend::loadFromFile(const QString &name,
                                          const QByteArray &key)
{
    QFile file(walletFilePath(name));
    if (!file.open(QIODevice::ReadOnly)) {
        qWarning("KWallet: Cannot open '%s' for reading",
                 qPrintable(file.fileName()));
        return false;
    }

    /* Read and verify header */
    char magic[4];
    if (file.read(magic, 4) != 4 ||
        memcmp(magic, WALLET_MAGIC, 4) != 0) {
        qWarning("KWallet: Invalid magic in '%s'", qPrintable(name));
        return false;
    }

    quint32 version;
    if (file.read(reinterpret_cast<char *>(&version), 4) != 4 ||
        version != WALLET_VERSION) {
        qWarning("KWallet: Unsupported version %u in '%s'",
                 version, qPrintable(name));
        return false;
    }

    /* Read encrypted payload */
    QByteArray ciphertext = file.readAll();
    file.close();

    /* Decrypt */
    QByteArray plaintext = decrypt(ciphertext, key);

    /* Deserialize */
    return deserialize(plaintext);
}

} /* namespace KWallet */
