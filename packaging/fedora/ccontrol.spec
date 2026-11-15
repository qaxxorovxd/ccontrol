Name:           ccontrol
Version:        0.2.0
Release:        1%{?dist}
Summary:        ccontrol desktop agent (control this PC from a paired Android phone)
License:        MIT OR Apache-2.0
URL:            https://github.com/ccontrol/ccontrol
Source0:        %{name}-%{version}.tar.gz
BuildRequires:  cargo >= 1.80, rust >= 1.80, gcc, systemd-rpm-macros
Recommends:     wireplumber
%global debug_package %{nil}

%description
Local-first agent that lets paired Android phones control this computer over
an authenticated TLS 1.3 LAN connection. Runs as a systemd *user* service.

%prep
%autosetup

%build
cargo build --release --locked -p ccontrol-agent

%install
install -Dm755 target/release/ccontrol-agent %{buildroot}%{_bindir}/ccontrol-agent
install -Dm644 packaging/linux/ccontrol-agent.service %{buildroot}%{_userunitdir}/ccontrol-agent.service
install -Dm644 packaging/linux/ccontrol-settings.desktop %{buildroot}%{_datadir}/applications/ccontrol-settings.desktop
install -Dm644 packaging/linux/ccontrol.svg %{buildroot}%{_datadir}/icons/hicolor/scalable/apps/ccontrol.svg

%check
cargo test --release --locked --workspace

%files
%{_bindir}/ccontrol-agent
%{_userunitdir}/ccontrol-agent.service
%{_datadir}/applications/ccontrol-settings.desktop
%{_datadir}/icons/hicolor/scalable/apps/ccontrol.svg

%changelog
* Sun Nov 15 2026 ccontrol maintainers - 0.2.0-1
- Phase 2: processes, app windows, now playing, disks/battery, self-unpair.
* Sun Nov 15 2026 ccontrol maintainers - 0.1.0-1
- Phase 1: pairing, trust store, capability manifest, volume/media/telemetry.
