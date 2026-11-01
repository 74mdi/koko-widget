# FreeBSD port for KOKO WIDGET.
#
#   cd /usr/ports/deskutils/koko-widget && make install clean
#
# gtk4-layer-shell is not in the FreeBSD tree, so the port builds the
# portable path: the app falls back to a plain Wayland toplevel, and X11
# still gets the full EWMH overlay.

PORTNAME=	koko-widget
DISTVERSION=	0.1.0
CATEGORIES=	deskutils
MASTER_SITES=	https://github.com/74mdi/koko-widget/archive/

MAINTAINER=	7amdi@tuta.io
COMMENT=	Native Liquid Glass floating widget overlay
WWW=		https://github.com/74mdi/koko-widget

LICENSE=	MIT
LICENSE_FILE=	${WRKSRC}/LICENSE

BUILD_DEPENDS=	pkgconf:devel/pkgconf
LIB_DEPENDS=	libgtk-4.so:x11-toolkits/gtk40 \
		libpango-1.0.so:x11-toolkits/pango \
		libcairo.so:graphics/cairo

USES=		cargo desktop-file-utils gnome pkgconfig
USE_GNOME=	gtk40
CARGO_BUILD=	yes
CARGO_INSTALL=	yes
CARGO_INSTALL_PATH=	.

CARGO_BUILD_ARGS=	--no-default-features

PLIST_FILES=	bin/koko-widget \
		share/applications/io.github.kokowidget.KokoWidget.desktop

post-install:
	${MKDIR} ${STAGEDIR}${PREFIX}/share/icons/hicolor
.for size in 16 24 32 48 64 128 256 512
	${MKDIR} ${STAGEDIR}${PREFIX}/share/icons/hicolor/${size}x${size}/apps
	${INSTALL_DATA} ${WRKSRC}/assets/icons/hicolor/${size}x${size}/apps/io.github.kokowidget.KokoWidget.png \
		${STAGEDIR}${PREFIX}/share/icons/hicolor/${size}x${size}/apps/io.github.kokowidget.KokoWidget.png
.endfor

.include <bsd.port.mk>
