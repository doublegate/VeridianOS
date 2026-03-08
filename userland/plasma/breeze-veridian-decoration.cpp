/*
 * VeridianOS -- breeze-veridian-decoration.cpp
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * KWin window decoration implementation using Breeze design language.
 *
 * Renders title bars with rounded corners, Breeze-colored window buttons
 * with hover/press animations, and configurable borders.  Integrates with
 * the VeridianOS color scheme system for light/dark mode support.
 */

#include "breeze-veridian-decoration.h"

#include <QDebug>
#include <QGuiApplication>
#include <QPainterPath>
#include <QScreen>
#include <QTextOption>
#include <QLinearGradient>
#include <QRadialGradient>

#include <cmath>

namespace Breeze {

/* ========================================================================= */
/* BreezeButton                                                              */
/* ========================================================================= */

BreezeButton::BreezeButton(KDecoration2::DecorationButtonType type,
                           KDecoration2::Decoration *decoration,
                           QObject *parent)
    : KDecoration2::DecorationButton(type, decoration, parent)
    , m_hoverOpacity(0.0)
    , m_pressOpacity(0.0)
    , m_hoverAnimation(new QPropertyAnimation(this, "hoverOpacity", this))
    , m_pressAnimation(new QPropertyAnimation(this, "pressOpacity", this))
{
    m_hoverAnimation->setDuration(DecorationMetrics::HoverDuration);
    m_hoverAnimation->setEasingCurve(QEasingCurve::InOutQuad);

    m_pressAnimation->setDuration(DecorationMetrics::PressDuration);
    m_pressAnimation->setEasingCurve(QEasingCurve::InOutQuad);

    setGeometry(QRectF(0, 0, DecorationMetrics::ButtonSize,
                        DecorationMetrics::ButtonSize));

    connect(this, &KDecoration2::DecorationButton::hoveredChanged,
            this, [this](bool hovered) {
        if (hovered) onHoverEntered(); else onHoverLeft();
    });
    connect(this, &KDecoration2::DecorationButton::pressedChanged,
            this, [this](bool pressed) {
        if (pressed) onPressed(); else onReleased();
    });
}

BreezeButton::~BreezeButton()
{
}

QPointer<BreezeButton> BreezeButton::create(
    KDecoration2::DecorationButtonType type,
    KDecoration2::Decoration *decoration,
    QObject *parent)
{
    return new BreezeButton(type, decoration, parent);
}

void BreezeButton::setHoverOpacity(qreal value)
{
    if (qFuzzyCompare(m_hoverOpacity, value))
        return;
    m_hoverOpacity = value;
    update();
}

void BreezeButton::setPressOpacity(qreal value)
{
    if (qFuzzyCompare(m_pressOpacity, value))
        return;
    m_pressOpacity = value;
    update();
}

QColor BreezeButton::baseColor() const
{
    switch (type()) {
    case KDecoration2::DecorationButtonType::Close:
        return QColor(218, 68, 83);     /* Breeze red */
    case KDecoration2::DecorationButtonType::Maximize:
        return QColor(39, 174, 96);     /* Breeze green */
    case KDecoration2::DecorationButtonType::Minimize:
        return QColor(246, 116, 0);     /* Breeze orange/yellow */
    case KDecoration2::DecorationButtonType::OnAllDesktops:
        return QColor(61, 174, 233);    /* Breeze blue */
    case KDecoration2::DecorationButtonType::KeepAbove:
    case KDecoration2::DecorationButtonType::KeepBelow:
    case KDecoration2::DecorationButtonType::Shade:
        return QColor(127, 140, 141);   /* Breeze gray */
    case KDecoration2::DecorationButtonType::ContextHelp:
        return QColor(155, 89, 182);    /* Breeze purple */
    default:
        return QColor(127, 140, 141);
    }
}

QColor BreezeButton::iconColor() const
{
    /* Icon is white on colored background when hovered/pressed,
     * title bar text color otherwise */
    if (m_hoverOpacity > 0.5 || m_pressOpacity > 0.5)
        return QColor(252, 252, 252);

    auto *deco = qobject_cast<BreezeDecoration *>(decoration());
    return deco ? deco->titleBarTextColor() : QColor(35, 38, 41);
}

void BreezeButton::paint(QPainter *painter, const QRect &repaintRegion)
{
    Q_UNUSED(repaintRegion);

    if (!decoration())
        return;

    painter->save();
    painter->setRenderHint(QPainter::Antialiasing, true);

    QRectF rect = geometry();
    QPointF center = rect.center();

    /* Background circle -- visible on hover/press */
    if (m_hoverOpacity > 0.0 || m_pressOpacity > 0.0) {
        QColor bg = baseColor();
        qreal opacity = qMax(m_hoverOpacity, m_pressOpacity);

        /* Darken on press */
        if (m_pressOpacity > 0.0)
            bg = bg.darker(120);

        bg.setAlphaF(opacity);

        painter->setPen(Qt::NoPen);
        painter->setBrush(bg);
        painter->drawEllipse(center, DecorationMetrics::ButtonHoverRadius,
                              DecorationMetrics::ButtonHoverRadius);
    }

    /* Icon */
    QRectF iconRect(center.x() - DecorationMetrics::ButtonIconSize / 2.0,
                    center.y() - DecorationMetrics::ButtonIconSize / 2.0,
                    DecorationMetrics::ButtonIconSize,
                    DecorationMetrics::ButtonIconSize);
    drawIcon(painter, iconRect);

    painter->restore();
}

void BreezeButton::drawIcon(QPainter *painter, const QRectF &rect) const
{
    switch (type()) {
    case KDecoration2::DecorationButtonType::Close:
        drawCloseIcon(painter, rect);
        break;
    case KDecoration2::DecorationButtonType::Maximize:
        drawMaximizeIcon(painter, rect);
        break;
    case KDecoration2::DecorationButtonType::Minimize:
        drawMinimizeIcon(painter, rect);
        break;
    case KDecoration2::DecorationButtonType::OnAllDesktops:
        drawOnAllDesktopsIcon(painter, rect);
        break;
    case KDecoration2::DecorationButtonType::KeepAbove:
        drawKeepAboveIcon(painter, rect);
        break;
    case KDecoration2::DecorationButtonType::KeepBelow:
        drawKeepBelowIcon(painter, rect);
        break;
    case KDecoration2::DecorationButtonType::Shade:
        drawShadeIcon(painter, rect);
        break;
    case KDecoration2::DecorationButtonType::ContextHelp:
        drawContextHelpIcon(painter, rect);
        break;
    default:
        break;
    }
}

void BreezeButton::drawCloseIcon(QPainter *painter, const QRectF &rect) const
{
    QPen pen(iconColor(), 1.5, Qt::SolidLine, Qt::RoundCap);
    painter->setPen(pen);
    painter->drawLine(rect.topLeft(), rect.bottomRight());
    painter->drawLine(rect.topRight(), rect.bottomLeft());
}

void BreezeButton::drawMaximizeIcon(QPainter *painter, const QRectF &rect) const
{
    QPen pen(iconColor(), 1.5, Qt::SolidLine, Qt::RoundCap, Qt::RoundJoin);
    painter->setPen(pen);
    painter->setBrush(Qt::NoBrush);

    if (isChecked()) {
        /* Restore: two overlapping rectangles */
        QRectF inner = rect.adjusted(0, 2, -2, 0);
        QRectF outer = rect.adjusted(2, 0, 0, -2);
        painter->drawRect(inner);
        painter->drawLine(outer.topLeft(), outer.topRight());
        painter->drawLine(outer.topRight(), outer.bottomRight());
    } else {
        /* Maximize: single rectangle */
        painter->drawRect(rect);
    }
}

void BreezeButton::drawMinimizeIcon(QPainter *painter, const QRectF &rect) const
{
    QPen pen(iconColor(), 1.5, Qt::SolidLine, Qt::RoundCap);
    painter->setPen(pen);
    qreal y = rect.bottom();
    painter->drawLine(QPointF(rect.left(), y), QPointF(rect.right(), y));
}

void BreezeButton::drawOnAllDesktopsIcon(QPainter *painter,
                                          const QRectF &rect) const
{
    painter->setPen(Qt::NoPen);
    painter->setBrush(iconColor());

    QPointF center = rect.center();
    qreal radius = isChecked() ? 3.5 : 2.5;
    painter->drawEllipse(center, radius, radius);
}

void BreezeButton::drawKeepAboveIcon(QPainter *painter,
                                      const QRectF &rect) const
{
    QPen pen(iconColor(), 1.5, Qt::SolidLine, Qt::RoundCap, Qt::RoundJoin);
    painter->setPen(pen);
    painter->setBrush(Qt::NoBrush);

    QPainterPath path;
    path.moveTo(rect.center().x(), rect.top());
    path.lineTo(rect.left(), rect.center().y());
    path.lineTo(rect.right(), rect.center().y());
    path.closeSubpath();
    painter->drawPath(path);
}

void BreezeButton::drawKeepBelowIcon(QPainter *painter,
                                      const QRectF &rect) const
{
    QPen pen(iconColor(), 1.5, Qt::SolidLine, Qt::RoundCap, Qt::RoundJoin);
    painter->setPen(pen);
    painter->setBrush(Qt::NoBrush);

    QPainterPath path;
    path.moveTo(rect.center().x(), rect.bottom());
    path.lineTo(rect.left(), rect.center().y());
    path.lineTo(rect.right(), rect.center().y());
    path.closeSubpath();
    painter->drawPath(path);
}

void BreezeButton::drawShadeIcon(QPainter *painter,
                                  const QRectF &rect) const
{
    QPen pen(iconColor(), 1.5, Qt::SolidLine, Qt::RoundCap);
    painter->setPen(pen);

    /* Horizontal line at top */
    painter->drawLine(QPointF(rect.left(), rect.top()),
                      QPointF(rect.right(), rect.top()));

    if (!isChecked()) {
        /* Down arrow below line */
        QPainterPath path;
        path.moveTo(rect.center().x(), rect.bottom());
        path.lineTo(rect.left() + 2, rect.center().y());
        path.lineTo(rect.right() - 2, rect.center().y());
        path.closeSubpath();
        painter->setBrush(iconColor());
        painter->setPen(Qt::NoPen);
        painter->drawPath(path);
    }
}

void BreezeButton::drawContextHelpIcon(QPainter *painter,
                                        const QRectF &rect) const
{
    QFont font = painter->font();
    font.setPixelSize(static_cast<int>(rect.height()));
    font.setBold(true);
    painter->setFont(font);
    painter->setPen(iconColor());
    painter->drawText(rect, Qt::AlignCenter, QStringLiteral("?"));
}

void BreezeButton::onHoverEntered()
{
    m_hoverAnimation->stop();
    m_hoverAnimation->setStartValue(m_hoverOpacity);
    m_hoverAnimation->setEndValue(1.0);
    m_hoverAnimation->start();
}

void BreezeButton::onHoverLeft()
{
    m_hoverAnimation->stop();
    m_hoverAnimation->setStartValue(m_hoverOpacity);
    m_hoverAnimation->setEndValue(0.0);
    m_hoverAnimation->start();
}

void BreezeButton::onPressed()
{
    m_pressAnimation->stop();
    m_pressAnimation->setStartValue(m_pressOpacity);
    m_pressAnimation->setEndValue(1.0);
    m_pressAnimation->start();
}

void BreezeButton::onReleased()
{
    m_pressAnimation->stop();
    m_pressAnimation->setStartValue(m_pressOpacity);
    m_pressAnimation->setEndValue(0.0);
    m_pressAnimation->start();
}

/* ========================================================================= */
/* BreezeDecoration                                                          */
/* ========================================================================= */

BreezeDecoration::BreezeDecoration(QObject *parent, const QVariantList &args)
    : KDecoration2::Decoration(parent, args)
    , m_borderSize(BorderSize::Normal)
    , m_activeOpacity(1.0)
    , m_activeAnimation(new QVariantAnimation(this))
    , m_leftButtons(nullptr)
    , m_rightButtons(nullptr)
{
    m_activeAnimation->setDuration(DecorationMetrics::ActiveChangeDuration);
    m_activeAnimation->setEasingCurve(QEasingCurve::InOutQuad);
    connect(m_activeAnimation, &QVariantAnimation::valueChanged,
            this, [this](const QVariant &value) {
        m_activeOpacity = value.toReal();
        update();
    });
}

BreezeDecoration::~BreezeDecoration()
{
}

void BreezeDecoration::init()
{
    loadConfiguration();

    auto *c = client();

    /* Connect signals */
    connect(c, &KDecoration2::DecoratedClient::activeChanged,
            this, &BreezeDecoration::onActiveChanged);
    connect(c, &KDecoration2::DecoratedClient::captionChanged,
            this, &BreezeDecoration::onCaptionChanged);
    connect(c, &KDecoration2::DecoratedClient::widthChanged,
            this, &BreezeDecoration::onWidthChanged);
    connect(c, &KDecoration2::DecoratedClient::heightChanged,
            this, &BreezeDecoration::onHeightChanged);

    /* Create buttons */
    createButtons();
    updateLayout();

    /* Generate shadow */
    setShadow(createShadow());
}

void BreezeDecoration::loadConfiguration()
{
    m_titleFont = QFont(QStringLiteral("Noto Sans"), DecorationMetrics::TitleBarFontSize);
    m_titleFont.setWeight(QFont::DemiBold);

    /* Border size from KWin settings (default: Normal) */
    auto *settings = this->settings().get();
    if (settings) {
        switch (settings->borderSize()) {
        case KDecoration2::BorderSize::None:
            m_borderSize = BorderSize::None;
            break;
        case KDecoration2::BorderSize::NoSides:
        case KDecoration2::BorderSize::Tiny:
            m_borderSize = BorderSize::Tiny;
            break;
        case KDecoration2::BorderSize::Normal:
            m_borderSize = BorderSize::Normal;
            break;
        case KDecoration2::BorderSize::Large:
            m_borderSize = BorderSize::Large;
            break;
        case KDecoration2::BorderSize::VeryLarge:
        case KDecoration2::BorderSize::Huge:
        case KDecoration2::BorderSize::VeryHuge:
        case KDecoration2::BorderSize::Oversized:
            m_borderSize = BorderSize::Huge;
            break;
        }
    }
}

int BreezeDecoration::borderWidth() const
{
    if (isMaximized())
        return 0;

    switch (m_borderSize) {
    case BorderSize::None:   return DecorationMetrics::BorderWidthNone;
    case BorderSize::Tiny:   return DecorationMetrics::BorderWidthTiny;
    case BorderSize::Normal: return DecorationMetrics::BorderWidthNormal;
    case BorderSize::Large:  return DecorationMetrics::BorderWidthLarge;
    case BorderSize::Huge:   return DecorationMetrics::BorderWidthHuge;
    }
    return DecorationMetrics::BorderWidthNormal;
}

int BreezeDecoration::titleBarHeight() const
{
    return static_cast<int>(DecorationMetrics::TitleBarHeight * scaleFactor());
}

bool BreezeDecoration::isMaximized() const
{
    auto *c = client();
    return c && (c->isMaximized() || c->isMaximizedHorizontally()
                 || c->isMaximizedVertically());
}

qreal BreezeDecoration::cornerRadius() const
{
    if (isMaximized())
        return DecorationMetrics::CornerRadiusMaximized;
    return DecorationMetrics::CornerRadius * scaleFactor();
}

qreal BreezeDecoration::scaleFactor() const
{
    /* Use primary screen DPI for scaling */
    QScreen *screen = QGuiApplication::primaryScreen();
    if (screen)
        return screen->devicePixelRatio();
    return 1.0;
}

QColor BreezeDecoration::titleBarColor() const
{
    auto *c = client();
    if (c && c->isActive())
        return c->palette().color(QPalette::Active, QPalette::Window);

    /* Blend between active and inactive based on animation */
    QColor active = c ? c->palette().color(QPalette::Active, QPalette::Window)
                      : QColor(227, 229, 231);
    QColor inactive = c ? c->palette().color(QPalette::Inactive, QPalette::Window)
                        : QColor(239, 240, 241);

    /* Linear interpolation */
    int r = static_cast<int>(inactive.red()   + m_activeOpacity * (active.red()   - inactive.red()));
    int g = static_cast<int>(inactive.green() + m_activeOpacity * (active.green() - inactive.green()));
    int b = static_cast<int>(inactive.blue()  + m_activeOpacity * (active.blue()  - inactive.blue()));

    return QColor(r, g, b);
}

QColor BreezeDecoration::titleBarTextColor() const
{
    auto *c = client();
    if (c && c->isActive())
        return c->palette().color(QPalette::Active, QPalette::WindowText);
    return c ? c->palette().color(QPalette::Inactive, QPalette::WindowText)
             : QColor(127, 140, 141);
}

QColor BreezeDecoration::borderColor() const
{
    return titleBarColor().darker(115);
}

QFont BreezeDecoration::titleBarFont() const
{
    return m_titleFont;
}

void BreezeDecoration::setBorderSize(BorderSize size)
{
    if (m_borderSize == size)
        return;
    m_borderSize = size;
    updateLayout();
    Q_EMIT borderSizeChanged();
}

/* ========================================================================= */
/* Painting                                                                  */
/* ========================================================================= */

void BreezeDecoration::paint(QPainter *painter, const QRect &repaintRegion)
{
    Q_UNUSED(repaintRegion);

    painter->save();
    painter->setRenderHint(QPainter::Antialiasing, true);

    paintTitleBar(painter);
    paintBorders(painter);
    paintCaption(painter);

    painter->restore();
}

void BreezeDecoration::paintTitleBar(QPainter *painter)
{
    QRectF titleRect(0, 0, size().width(), titleBarHeight());
    QColor bg = titleBarColor();
    qreal radius = cornerRadius();

    /* Draw title bar with rounded top corners */
    QPainterPath path;
    if (radius > 0) {
        path.moveTo(0, titleRect.bottom());
        path.lineTo(0, radius);
        path.quadTo(0, 0, radius, 0);
        path.lineTo(titleRect.right() - radius, 0);
        path.quadTo(titleRect.right(), 0, titleRect.right(), radius);
        path.lineTo(titleRect.right(), titleRect.bottom());
        path.closeSubpath();
    } else {
        path.addRect(titleRect);
    }

    painter->setPen(Qt::NoPen);
    painter->setBrush(bg);
    painter->drawPath(path);

    /* Subtle bottom separator line */
    painter->setPen(QPen(borderColor(), 1.0));
    painter->drawLine(QPointF(0, titleRect.bottom()),
                      QPointF(titleRect.right(), titleRect.bottom()));
}

void BreezeDecoration::paintBorders(QPainter *painter)
{
    int bw = borderWidth();
    if (bw <= 0)
        return;

    QColor bc = borderColor();
    painter->setPen(Qt::NoPen);
    painter->setBrush(bc);

    qreal w = size().width();
    qreal h = size().height();
    int tbh = titleBarHeight();

    /* Left border */
    painter->drawRect(QRectF(0, tbh, bw, h - tbh));

    /* Right border */
    painter->drawRect(QRectF(w - bw, tbh, bw, h - tbh));

    /* Bottom border */
    qreal radius = cornerRadius();
    if (radius > 0) {
        QPainterPath path;
        path.moveTo(0, h - radius);
        path.quadTo(0, h, radius, h);
        path.lineTo(w - radius, h);
        path.quadTo(w, h, w, h - radius);
        path.lineTo(w, h - bw);
        path.lineTo(0, h - bw);
        path.closeSubpath();
        painter->drawPath(path);
    } else {
        painter->drawRect(QRectF(0, h - bw, w, bw));
    }
}

void BreezeDecoration::paintCaption(QPainter *painter)
{
    auto *c = client();
    if (!c)
        return;

    QString caption = c->caption();
    if (caption.isEmpty())
        return;

    QRectF titleRect(0, 0, size().width(), titleBarHeight());

    /* Calculate available space between button groups */
    qreal leftEdge = DecorationMetrics::TitleBarSideMargin;
    qreal rightEdge = titleRect.width() - DecorationMetrics::TitleBarSideMargin;

    if (m_leftButtons && m_leftButtons->geometry().isValid())
        leftEdge = m_leftButtons->geometry().right() + DecorationMetrics::ButtonSpacing;
    if (m_rightButtons && m_rightButtons->geometry().isValid())
        rightEdge = m_rightButtons->geometry().left() - DecorationMetrics::ButtonSpacing;

    QRectF textRect(leftEdge, 0, rightEdge - leftEdge, titleBarHeight());

    painter->setPen(titleBarTextColor());
    painter->setFont(m_titleFont);

    /* Elide long captions */
    QFontMetrics fm(m_titleFont);
    QString elidedCaption = fm.elidedText(caption, Qt::ElideRight,
                                           static_cast<int>(textRect.width()));

    QTextOption textOption(Qt::AlignVCenter | Qt::AlignCenter);
    textOption.setWrapMode(QTextOption::NoWrap);
    painter->drawText(textRect, elidedCaption, textOption);
}

/* ========================================================================= */
/* Shadow                                                                    */
/* ========================================================================= */

QSharedPointer<KDecoration2::DecorationShadow> BreezeDecoration::createShadow()
{
    QImage shadowImage = generateShadowImage();

    auto shadow = QSharedPointer<KDecoration2::DecorationShadow>::create();
    shadow->setShadow(shadowImage);

    int shadowSize = DecorationMetrics::ShadowSize;
    shadow->setPadding(QMargins(shadowSize, shadowSize,
                                shadowSize, shadowSize + DecorationMetrics::ShadowOffset));
    shadow->setInnerShadowRect(QRect(shadowSize, shadowSize,
                                      shadowImage.width() - 2 * shadowSize,
                                      shadowImage.height() - 2 * shadowSize));

    return shadow;
}

QImage BreezeDecoration::generateShadowImage() const
{
    int size = DecorationMetrics::ShadowSize * 2 + 1;
    QImage image(size, size, QImage::Format_ARGB32_Premultiplied);
    image.fill(Qt::transparent);

    QPainter painter(&image);
    painter.setRenderHint(QPainter::Antialiasing, true);

    QPointF center(size / 2.0, size / 2.0 - DecorationMetrics::ShadowOffset);
    QRadialGradient gradient(center, DecorationMetrics::ShadowSize);
    gradient.setColorAt(0.0, QColor(0, 0, 0, static_cast<int>(255 * DecorationMetrics::ShadowStrength)));
    gradient.setColorAt(0.3, QColor(0, 0, 0, static_cast<int>(128 * DecorationMetrics::ShadowStrength)));
    gradient.setColorAt(1.0, Qt::transparent);

    painter.setPen(Qt::NoPen);
    painter.setBrush(gradient);
    painter.drawEllipse(center, DecorationMetrics::ShadowSize,
                        DecorationMetrics::ShadowSize);

    return image;
}

/* ========================================================================= */
/* Button management                                                         */
/* ========================================================================= */

void BreezeDecoration::createButtons()
{
    m_leftButtons = new KDecoration2::DecorationButtonGroup(
        KDecoration2::DecorationButtonGroup::Position::Left, this);
    m_rightButtons = new KDecoration2::DecorationButtonGroup(
        KDecoration2::DecorationButtonGroup::Position::Right, this);

    auto *settings = this->settings().get();
    if (!settings)
        return;

    /* Left buttons (typically: Menu) */
    for (auto type : settings->decorationButtonsLeft()) {
        auto *button = BreezeButton::create(type, this, m_leftButtons);
        if (button)
            m_leftButtons->addButton(button);
    }

    /* Right buttons (typically: Minimize, Maximize, Close) */
    for (auto type : settings->decorationButtonsRight()) {
        auto *button = BreezeButton::create(type, this, m_rightButtons);
        if (button)
            m_rightButtons->addButton(button);
    }
}

void BreezeDecoration::updateButtonPositions()
{
    if (!m_leftButtons || !m_rightButtons)
        return;

    int tbh = titleBarHeight();
    qreal buttonY = (tbh - DecorationMetrics::ButtonSize) / 2.0;

    m_leftButtons->setPos(QPointF(DecorationMetrics::TitleBarSideMargin, buttonY));
    m_leftButtons->setSpacing(DecorationMetrics::ButtonSpacing);

    /* Right buttons are positioned from the right edge */
    qreal rightX = size().width() - DecorationMetrics::TitleBarSideMargin
                 - m_rightButtons->geometry().width();
    m_rightButtons->setPos(QPointF(rightX, buttonY));
    m_rightButtons->setSpacing(DecorationMetrics::ButtonSpacing);
}

void BreezeDecoration::updateLayout()
{
    int tbh = titleBarHeight();
    int bw  = borderWidth();

    setBorders(QMargins(bw, tbh, bw, bw));
    updateButtonPositions();
    update();
}

/* ========================================================================= */
/* Signal handlers                                                           */
/* ========================================================================= */

void BreezeDecoration::onActiveChanged()
{
    m_activeAnimation->stop();
    m_activeAnimation->setStartValue(m_activeOpacity);
    m_activeAnimation->setEndValue(client()->isActive() ? 1.0 : 0.0);
    m_activeAnimation->start();
}

void BreezeDecoration::onCaptionChanged()
{
    update();
}

void BreezeDecoration::onMaximizedChanged()
{
    updateLayout();
}

void BreezeDecoration::onWidthChanged()
{
    updateButtonPositions();
    update();
}

void BreezeDecoration::onHeightChanged()
{
    update();
}

void BreezeDecoration::onShadingChanged()
{
    updateLayout();
}

/* ========================================================================= */
/* Plugin factory                                                            */
/* ========================================================================= */

BreezeDecorationPlugin::BreezeDecorationPlugin(QObject *parent)
    : QObject(parent)
{
}

BreezeDecorationPlugin::~BreezeDecorationPlugin()
{
}

} /* namespace Breeze */
