/*
 * VeridianOS -- qveridianmain.cpp
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Plugin entry point of the "veridian" platform: the factory Qt asks for a
 * QPlatformIntegration when an application runs with -platform veridian or
 * QT_QPA_PLATFORM=veridian. The class name must match the CMake plugin
 * target (QVeridianIntegrationPlugin), which a static Qt imports by name.
 */

#include <QtGui/qpa/qplatformintegrationplugin.h>

#include "qveridianintegration.h"

QT_BEGIN_NAMESPACE

class QVeridianIntegrationPlugin : public QPlatformIntegrationPlugin
{
    Q_OBJECT
    Q_PLUGIN_METADATA(IID QPlatformIntegrationFactoryInterface_iid FILE "veridian.json")

public:
    QPlatformIntegration *create(const QString &system, const QStringList &paramList) override
    {
        if (!system.compare(QLatin1String("veridian"), Qt::CaseInsensitive))
            return new QVeridianIntegration(paramList);
        return nullptr;
    }
};

QT_END_NAMESPACE

#include "qveridianmain.moc"
