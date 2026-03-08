/*
 * VeridianOS -- kwallet-veridian-backend.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * KWallet backend for VeridianOS.  Provides a simple file-based
 * credential storage for the KDE Wallet framework.  Wallets are
 * stored as encrypted files in ~/.local/share/kwallet/.
 *
 * NOTE: This initial implementation uses XOR-based obfuscation.
 * A production deployment should replace this with proper AES-256-GCM
 * encryption.  The API is identical; only the encrypt/decrypt
 * internals need updating.
 */

#ifndef KWALLET_VERIDIAN_BACKEND_H
#define KWALLET_VERIDIAN_BACKEND_H

#include <QObject>
#include <QString>
#include <QStringList>
#include <QByteArray>
#include <QMap>
#include <QHash>

namespace KWallet {

/* ========================================================================= */
/* Wallet entry types                                                        */
/* ========================================================================= */

enum EntryType {
    Unknown   = 0,
    Password  = 1,
    Stream    = 2,
    Map       = 3
};

/* ========================================================================= */
/* WalletEntry                                                               */
/* ========================================================================= */

struct WalletEntry {
    QString key;
    QByteArray value;
    EntryType type;
};

/* ========================================================================= */
/* WalletFolder                                                              */
/* ========================================================================= */

struct WalletFolder {
    QString name;
    QMap<QString, WalletEntry> entries;
};

/* ========================================================================= */
/* VeridianWalletBackend                                                     */
/* ========================================================================= */

/**
 * File-based KWallet backend for VeridianOS.
 *
 * Storage layout:
 *   ~/.local/share/kwallet/<walletname>.kwl  -- encrypted wallet file
 *
 * File format (version 1):
 *   [4 bytes]  magic "VKWL"
 *   [4 bytes]  version (1)
 *   [4 bytes]  number of folders
 *   For each folder:
 *     [4 bytes]  folder name length
 *     [N bytes]  folder name (UTF-8)
 *     [4 bytes]  number of entries
 *     For each entry:
 *       [4 bytes]  key length
 *       [N bytes]  key (UTF-8)
 *       [4 bytes]  entry type (Password=1, Stream=2, Map=3)
 *       [4 bytes]  value length
 *       [N bytes]  value data
 *
 * The entire payload after the header is XOR-encrypted with a key
 * derived from the wallet password.  (TODO: replace with AES-256-GCM)
 */
class VeridianWalletBackend : public QObject
{
    Q_OBJECT

public:
    explicit VeridianWalletBackend(QObject *parent = nullptr);
    ~VeridianWalletBackend() override;

    /* ----- Wallet lifecycle ----- */

    /**
     * Open (or create) a named wallet.
     *
     * If the wallet file exists, it is decrypted and loaded into memory.
     * If it does not exist, a new empty wallet is created.
     *
     * @param name     Wallet name (e.g., "kdewallet").
     * @param password Password for encryption/decryption.
     * @return 0 on success, -1 on error.
     */
    int openWallet(const QString &name, const QString &password);

    /**
     * Close the currently open wallet, writing changes to disk.
     *
     * @return 0 on success, -1 on error.
     */
    int closeWallet();

    /**
     * Check if a wallet is currently open.
     */
    bool isOpen() const;

    /**
     * Get the name of the currently open wallet.
     */
    QString currentWallet() const;

    /**
     * List all available wallet files.
     */
    QStringList wallets() const;

    /**
     * Delete a wallet file from disk.
     */
    int deleteWallet(const QString &name);

    /* ----- Folder operations ----- */

    /**
     * List folders in the current wallet.
     */
    QStringList folderList() const;

    /**
     * Check if a folder exists.
     */
    bool hasFolder(const QString &folder) const;

    /**
     * Create a new folder.
     */
    bool createFolder(const QString &folder);

    /**
     * Remove a folder and all its entries.
     */
    bool removeFolder(const QString &folder);

    /**
     * Set the current working folder.
     */
    void setFolder(const QString &folder);

    /**
     * Get the current working folder.
     */
    QString currentFolder() const;

    /* ----- Entry operations ----- */

    /**
     * Read a password entry.
     *
     * @param key      Entry key.
     * @param value    Output: the password string.
     * @return 0 on success, -1 if not found.
     */
    int readPassword(const QString &key, QString &value) const;

    /**
     * Write a password entry.
     *
     * @param key      Entry key.
     * @param value    The password string.
     * @return 0 on success, -1 on error.
     */
    int writePassword(const QString &key, const QString &value);

    /**
     * Read a binary data entry.
     */
    int readEntry(const QString &key, QByteArray &value) const;

    /**
     * Write a binary data entry.
     */
    int writeEntry(const QString &key, const QByteArray &value);

    /**
     * Read a map entry (key=value pairs serialized as QByteArray).
     */
    int readMap(const QString &key, QMap<QString, QString> &value) const;

    /**
     * Write a map entry.
     */
    int writeMap(const QString &key, const QMap<QString, QString> &value);

    /**
     * Check if an entry exists in the current folder.
     */
    bool hasEntry(const QString &key) const;

    /**
     * Remove an entry from the current folder.
     */
    int removeEntry(const QString &key);

    /**
     * Rename an entry.
     */
    int renameEntry(const QString &oldKey, const QString &newKey);

    /**
     * List all entry keys in the current folder.
     */
    QStringList entryList() const;

    /**
     * Get the type of an entry.
     */
    EntryType entryType(const QString &key) const;

Q_SIGNALS:
    void walletOpened(const QString &name);
    void walletClosed();
    void folderUpdated(const QString &folder);

private:
    /* ----- Encryption ----- */
    QByteArray deriveKey(const QString &password) const;
    QByteArray encrypt(const QByteArray &data, const QByteArray &key) const;
    QByteArray decrypt(const QByteArray &data, const QByteArray &key) const;

    /* ----- Serialization ----- */
    QByteArray serialize() const;
    bool deserialize(const QByteArray &data);

    /* ----- File I/O ----- */
    QString walletFilePath(const QString &name) const;
    QString walletDirectory() const;
    bool saveToFile();
    bool loadFromFile(const QString &name, const QByteArray &key);

    /* ----- State ----- */
    QString m_walletName;
    QByteArray m_encryptionKey;
    QString m_currentFolder;
    QHash<QString, WalletFolder> m_folders;
    bool m_open;
    bool m_dirty;
};

} /* namespace KWallet */

#endif /* KWALLET_VERIDIAN_BACKEND_H */
