# RPM spec for KOKO WIDGET.
#
#   rpmbuild -ba packaging/fedora/koko-widget.spec
#
# Builds against the system GTK4 and layer-shell; the layer-shell build is
# optional, so the spec does not hard-require it.

Name:           koko-widget
Version:        0.1.0
Release:        1%{?dist}
Summary:        Native Liquid Glass floating widget overlay

License:        MIT
URL:            https://github.com/74mdi/koko-widget
Source0:        %{url}/archive/v%{version}/%{name}-%{version}.tar.gz

BuildRequires:  rust >= 1.92
BuildRequires:  cargo
BuildRequires:  pkgconfig(gtk4) >= 4.18
BuildRequires:  pkgconfig(pango)
BuildRequires:  pkgconfig(cairo)
BuildRequires:  pkgconfig(gtk4-layer-shell) >= 0.8
BuildRequires:  desktop-file-utils

Requires:       gtk4 >= 4.18
Requires:       pango
Recommends:     gtk4-layer-shell

%description
A floating widget layer for the desktop. It shows the currently playing
track from any MPRIS-compliant player, the time, the date and any text you
want, each as an independent glass pill that can be moved, resized and
restyled.

Written in Rust with GTK4 and Cairo. No Electron, no Tauri, no WebView.
Layer-shell compositors get anchored surfaces; X11 uses EWMH dock windows.

%prep
%autosetup

%build
export CARGO_BUILD_JOBS=1
%cargo_build --all-features

%install
install -Dm755 %{_builddir}/%{name}-%{version}/target/release/koko-widget \
    %{buildroot}%{_bindir}/koko-widget
install -Dm644 data/io.github.kokowidget.KokoWidget.desktop \
    %{buildroot}%{_datadir}/applications/io.github.kokowidget.KokoWidget.desktop
for size in 16 24 32 48 64 128 256 512; do
    install -Dm644 "assets/icons/hicolor/${size}x${size}/apps/io.github.kokowidget.KokoWidget.png" \
        "%{buildroot}%{_datadir}/icons/hicolor/${size}x${size}/apps/io.github.kokowidget.KokoWidget.png"
done

%check
desktop-file-validate %{buildroot}%{_datadir}/applications/io.github.kokowidget.KokoWidget.desktop

%files
%license LICENSE
%{_bindir}/koko-widget
%{_datadir}/applications/io.github.kokowidget.KokoWidget.desktop
%{_datadir}/icons/hicolor/*/apps/io.github.kokowidget.KokoWidget.png

%changelog
* Sun Nov 01 2026 74mdi <7amdi@tuta.io> - 0.1.0-1
- Initial package
