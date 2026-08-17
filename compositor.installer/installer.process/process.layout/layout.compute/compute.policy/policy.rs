/// xdg-desktop-portal preference (modeled on .reference/references/installer-artifact/xdg-portal).
pub fn portals_conf() -> String {
    "[preferred]\ndefault=gtk\n".to_string()
}

/// PAM service for the lock screen (from installation-y5-lock). Used only if the
/// staged template file is absent.
pub fn pam_y5_lock() -> String {
    "# /etc/pam.d/y5-lock\n\
     # PAM service for the y5 compositor's lock screen.\n\
     # RHEL / Fedora / Arch / openSUSE:\n\
     auth     include    system-auth\n\
     account  include    system-auth\n"
        .to_string()
}

/// App-launcher entry for the developer tool window (installed to
/// /usr/share/applications). `Exec`/`StartupWMClass`/`Icon` use the bare
/// `y5.compositor.monitor` command (on PATH; launched under that name, the GTK window
/// reports it as its WM class). The app self-sets WEBKIT_DISABLE_DMABUF_RENDERER, so no
/// env wrapper is needed.
pub fn devtool_desktop_entry() -> String {
    "[Desktop Entry]\n\
     Categories=\n\
     Comment=y5 developer log viewer (Tauri + React)\n\
     Exec=y5.compositor.monitor\n\
     StartupWMClass=y5.compositor.monitor\n\
     Icon=y5.compositor.monitor\n\
     Name=y5.compositor.monitor\n\
     Terminal=false\n\
     Type=Application\n"
        .to_string()
}

/// systemd user service for the polkit authentication agent (new — none shipped).
pub fn polkit_service() -> String {
    "# ~/.config/systemd/user/y5-polkit-agent.service\n\
     [Unit]\n\
     Description=Y5 polkit authentication agent\n\
     After=graphical-session.target\n\
     PartOf=graphical-session.target\n\
     \n\
     [Service]\n\
     Type=simple\n\
     ExecStart=/usr/local/bin/y5-polkit-agent\n\
     Restart=on-failure\n\
     \n\
     [Install]\n\
     WantedBy=graphical-session.target\n"
        .to_string()
}
