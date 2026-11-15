import AppKit
import LauncherCore
import WebKit

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate, WKNavigationDelegate, WKUIDelegate {
    private let worker = WorkerController()
    private let store: PayloadStore
    private var window: NSWindow!
    private var webView: WKWebView!
    private var activePayload: Payload?
    private var updateCheckRunning = false

    private static let releasesPage = URL(string: "https://github.com/wendibus/reco-minimap/releases/latest")!

    override init() {
        let version = Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "0.0.0"
        store = PayloadStore(
            installRoot: PayloadStore.defaultInstallRoot,
            // RECO_MINIMAP_PAYLOAD_DIR lets `swift run` use a payload folder built by mac/scripts/build-payload.sh.
            bundledRoot: ProcessInfo.processInfo.environment["RECO_MINIMAP_PAYLOAD_DIR"].map { URL(fileURLWithPath: $0, isDirectory: true) }
                ?? Bundle.main.resourceURL?.appendingPathComponent("payload", isDirectory: true),
            launcherVersion: version
        )
        super.init()
    }

    // MARK: Lifecycle

    func applicationDidFinishLaunching(_ notification: Notification) {
        buildMenu()
        buildWindow()
        NSApp.activate(ignoringOtherApps: true)
        Task { await launchWorkerAndLoad() }
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { true }

    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        guard worker.isRunning else { return .terminateNow }
        Task { @MainActor in
            var proceed = true
            if await worker.isBusy() {
                proceed = confirm(
                    L("Es läuft gerade eine Aufgabe (zum Beispiel eine Verarbeitung oder der Live-Modus). Beenden bricht sie ab.",
                      "A task is still running (for example processing or live mode). Quitting will cancel it.",
                      "Une tâche est en cours (par exemple un traitement ou le mode direct). Quitter l'annulera.",
                      "Hay una tarea en curso (por ejemplo, un procesamiento o el modo en directo). Salir la cancelará."),
                    ok: L("Trotzdem beenden", "Quit anyway", "Quitter quand même", "Salir de todos modos"),
                    cancel: L("Weiterlaufen lassen", "Keep running", "Laisser tourner", "Dejar en marcha")
                )
            }
            NSApp.reply(toApplicationShouldTerminate: proceed)
        }
        return .terminateLater
    }

    func applicationWillTerminate(_ notification: Notification) {
        worker.stop()
    }

    // MARK: Starting

    private func launchWorkerAndLoad() async {
        _ = try? store.seedBundledPayloadIfNeeded()
        var lastError: Error?
        for _ in 0..<3 {
            guard let payload = store.activePayload() else { break }
            do {
                try await worker.start(payload: payload)
                activePayload = payload
                loadInterface()
                store.prune(keeping: 2, protecting: [payload.version])
                Task { await checkForUpdates(userInitiated: false) }
                return
            } catch {
                lastError = error
                if case WorkerError.programMissing = error { break }
                // This payload does not start: remember that and fall back to an older one.
                store.markBad(version: payload.version)
            }
        }
        showStartFailure(lastError)
    }

    private func loadInterface() {
        guard let url = URL(string: "\(worker.baseURL.absoluteString)/?launcher=1") else { return }
        webView.load(URLRequest(url: url))
    }

    private func showStartFailure(_ error: Error?) {
        let alert = NSAlert()
        alert.alertStyle = .critical
        alert.messageText = L("Reco Minimap konnte nicht starten", "Reco Minimap could not start", "Reco Minimap n'a pas pu démarrer", "Reco Minimap no pudo iniciarse")
        alert.informativeText = error?.localizedDescription
            ?? L("Es wurde keine startbare Programmversion gefunden. Bitte Reco Minimap neu installieren.",
                 "No runnable program version was found. Please reinstall Reco Minimap.",
                 "Aucune version exécutable du programme n'a été trouvée. Veuillez réinstaller Reco Minimap.",
                 "No se encontró ninguna versión ejecutable del programa. Vuelve a instalar Reco Minimap.")
        alert.addButton(withTitle: L("Beenden", "Quit", "Quitter", "Salir"))
        alert.addButton(withTitle: L("Protokoll zeigen", "Show log", "Afficher le journal", "Mostrar el registro"))
        if alert.runModal() == .alertSecondButtonReturn { showLog() }
        NSApp.terminate(nil)
    }

    // MARK: Updates

    /// The standard About panel with what is new in the running version (from the program's own
    /// notes, in the system language).
    @objc private func showAbout() {
        Task { @MainActor in
            var options: [NSApplication.AboutPanelOptionKey: Any] = [:]
            if worker.isRunning, let (version, lines) = await currentNotes() {
                options[.applicationVersion] = version
                let head = L("Neu in \(version):", "New in \(version):", "Nouveautés de la \(version) :", "Novedades de la \(version):")
                let text = head + "\n" + lines.map { "• " + $0 }.joined(separator: "\n")
                let style = NSMutableParagraphStyle()
                style.paragraphSpacing = 4
                options[.credits] = NSAttributedString(string: text, attributes: [
                    .font: NSFont.systemFont(ofSize: NSFont.smallSystemFontSize),
                    .foregroundColor: NSColor.labelColor,
                    .paragraphStyle: style,
                ])
            }
            NSApp.orderFrontStandardAboutPanel(options: options)
            NSApp.activate(ignoringOtherApps: true)
        }
    }

    private func currentNotes() async -> (String, [String])? {
        guard let url = URL(string: "\(worker.baseURL.absoluteString)/api/changelog"),
              let (data, _) = try? await URLSession.shared.data(from: url),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let version = object["version"] as? String,
              let notes = object["notes"] as? [String: Any],
              let current = notes[version] as? [String: Any]
        else { return nil }
        let lang = String((Locale.preferredLanguages.first ?? "en").prefix(2))
        let lines = (current[lang] as? [String]) ?? (current["en"] as? [String]) ?? []
        return lines.isEmpty ? nil : (version, lines)
    }

    @objc private func checkForUpdatesFromMenu() {
        Task { await checkForUpdates(userInitiated: true) }
    }

    private func checkForUpdates(userInitiated: Bool) async {
        guard !updateCheckRunning, let installed = activePayload?.version else { return }
        updateCheckRunning = true
        defer { updateCheckRunning = false }
        do {
            let release = try await Updater.fetchLatestRelease()
            switch UpdatePlanner.decide(release: release, installedVersion: installed, allowFileURLs: Updater.allowsFileURLs) {
            case .upToDate:
                if userInitiated {
                    inform(L("Reco Minimap ist auf dem neuesten Stand (Version \(installed)).",
                             "Reco Minimap is up to date (version \(installed)).",
                             "Reco Minimap est à jour (version \(installed)).",
                             "Reco Minimap está actualizado (versión \(installed))."))
                }
            case .needsNewApp(let version, let page):
                let key = "notifiedNeedsNewApp"
                if !userInitiated && UserDefaults.standard.string(forKey: key) == version { return }
                UserDefaults.standard.set(version, forKey: key)
                if confirm(
                    L("Version \(version) ist erschienen und braucht eine neue App. Die Download-Seite öffnen?",
                      "Version \(version) is available and needs a new app download. Open the download page?",
                      "La version \(version) est disponible et nécessite une nouvelle app. Ouvrir la page de téléchargement ?",
                      "La versión \(version) está disponible y necesita una app nueva. ¿Abrir la página de descarga?"),
                    ok: L("Seite öffnen", "Open page", "Ouvrir la page", "Abrir la página"), cancel: L("Später", "Later", "Plus tard", "Más tarde")
                ) { NSWorkspace.shared.open(page) }
            case .available(let version, let zip, let signature):
                guard Updater.isConfigured else {
                    if userInitiated { inform(UpdaterError.notConfigured.localizedDescription) }
                    return
                }
                guard confirm(
                    L("Version \(version) ist verfügbar. Jetzt installieren?", "Version \(version) is available. Install now?", "La version \(version) est disponible. L'installer maintenant ?", "La versión \(version) está disponible. ¿Instalarla ahora?"),
                    ok: L("Installieren", "Install", "Installer", "Instalar"), cancel: L("Später", "Later", "Plus tard", "Más tarde")
                ) else { return }
                if await worker.isBusy() {
                    inform(L("Es läuft gerade eine Aufgabe. Bitte das Update nach deren Ende installieren.",
                             "A task is running. Please install the update after it has finished.",
                             "Une tâche est en cours. Veuillez installer la mise à jour une fois qu'elle est terminée.",
                             "Hay una tarea en curso. Instala la actualización cuando haya terminado."))
                    return
                }
                window.title = L("Reco Minimap – installiere Update …", "Reco Minimap – installing update …", "Reco Minimap – installation de la mise à jour …", "Reco Minimap – instalando la actualización …")
                defer { window.title = "Reco Minimap" }
                let payload = try await Updater.install(zip: zip, signature: signature, into: store)
                await restart(with: payload)
            }
        } catch PayloadError.launcherTooOld(let required) {
            if confirm(
                L("Dieses Update braucht Reco Minimap \(required) oder neuer. Die Download-Seite öffnen?",
                  "This update needs Reco Minimap \(required) or newer. Open the download page?",
                  "Cette mise à jour nécessite Reco Minimap \(required) ou plus récent. Ouvrir la page de téléchargement ?",
                  "Esta actualización necesita Reco Minimap \(required) o posterior. ¿Abrir la página de descarga?"),
                ok: L("Seite öffnen", "Open page", "Ouvrir la page", "Abrir la página"), cancel: L("Später", "Later", "Plus tard", "Más tarde")
            ) { NSWorkspace.shared.open(Self.releasesPage) }
        } catch {
            if userInitiated {
                inform(L("Die Update-Suche ist fehlgeschlagen: ", "Checking for updates failed: ", "La recherche de mises à jour a échoué : ", "Error al buscar actualizaciones: ") + error.localizedDescription)
            }
        }
    }

    private func restart(with payload: Payload) async {
        worker.stop()
        do {
            try await worker.start(payload: payload)
            activePayload = payload
            loadInterface()
        } catch {
            store.markBad(version: payload.version)
            await launchWorkerAndLoad()
        }
    }

    // MARK: Window and menu

    private func buildWindow() {
        let configuration = WKWebViewConfiguration()
        webView = WKWebView(frame: .zero, configuration: configuration)
        webView.navigationDelegate = self
        webView.uiDelegate = self
        if #available(macOS 13.3, *) { webView.isInspectable = true }

        window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 1360, height: 900),
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered,
            defer: false
        )
        window.title = "Reco Minimap"
        window.contentView = webView
        window.center()
        window.setFrameAutosaveName("RecoMinimapMainWindow")
        window.makeKeyAndOrderFront(nil)
        webView.loadHTMLString(
            "<body style='font-family:-apple-system;display:flex;height:100vh;align-items:center;justify-content:center;color:#667085'>"
                + "<div>\(L("Reco Minimap startet …", "Starting Reco Minimap …", "Démarrage de Reco Minimap …", "Iniciando Reco Minimap …"))</div></body>",
            baseURL: nil
        )
    }

    private func buildMenu() {
        let main = NSMenu()

        let appItem = NSMenuItem()
        let appMenu = NSMenu()
        let about = appMenu.addItem(withTitle: L("Über Reco Minimap", "About Reco Minimap", "À propos de Reco Minimap", "Acerca de Reco Minimap"),
                                    action: #selector(showAbout), keyEquivalent: "")
        about.target = self
        let updates = appMenu.addItem(withTitle: L("Nach Updates suchen …", "Check for Updates…", "Rechercher des mises à jour…", "Buscar actualizaciones…"),
                                      action: #selector(checkForUpdatesFromMenu), keyEquivalent: "")
        updates.target = self
        appMenu.addItem(.separator())
        appMenu.addItem(withTitle: L("Reco Minimap ausblenden", "Hide Reco Minimap", "Masquer Reco Minimap", "Ocultar Reco Minimap"),
                        action: #selector(NSApplication.hide(_:)), keyEquivalent: "h")
        appMenu.addItem(.separator())
        appMenu.addItem(withTitle: L("Reco Minimap beenden", "Quit Reco Minimap", "Quitter Reco Minimap", "Salir de Reco Minimap"),
                        action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
        appItem.submenu = appMenu
        main.addItem(appItem)

        let editItem = NSMenuItem()
        let editMenu = NSMenu(title: L("Bearbeiten", "Edit", "Édition", "Edición"))
        editMenu.addItem(withTitle: L("Widerrufen", "Undo", "Annuler", "Deshacer"), action: Selector(("undo:")), keyEquivalent: "z")
        let redo = editMenu.addItem(withTitle: L("Wiederholen", "Redo", "Rétablir", "Rehacer"), action: Selector(("redo:")), keyEquivalent: "z")
        redo.keyEquivalentModifierMask = [.command, .shift]
        editMenu.addItem(.separator())
        editMenu.addItem(withTitle: L("Ausschneiden", "Cut", "Couper", "Cortar"), action: #selector(NSText.cut(_:)), keyEquivalent: "x")
        editMenu.addItem(withTitle: L("Kopieren", "Copy", "Copier", "Copiar"), action: #selector(NSText.copy(_:)), keyEquivalent: "c")
        editMenu.addItem(withTitle: L("Einsetzen", "Paste", "Coller", "Pegar"), action: #selector(NSText.paste(_:)), keyEquivalent: "v")
        editMenu.addItem(withTitle: L("Alles auswählen", "Select All", "Tout sélectionner", "Seleccionar todo"), action: #selector(NSText.selectAll(_:)), keyEquivalent: "a")
        editItem.submenu = editMenu
        main.addItem(editItem)

        let viewItem = NSMenuItem()
        let viewMenu = NSMenu(title: L("Ansicht", "View", "Présentation", "Visualización"))
        let reload = viewMenu.addItem(withTitle: L("Neu laden", "Reload", "Recharger", "Volver a cargar"), action: #selector(reloadInterface), keyEquivalent: "r")
        reload.target = self
        let zoomIn = viewMenu.addItem(withTitle: L("Vergrößern", "Zoom In", "Zoom avant", "Ampliar"), action: #selector(zoomIn), keyEquivalent: "+")
        zoomIn.target = self
        let zoomOut = viewMenu.addItem(withTitle: L("Verkleinern", "Zoom Out", "Zoom arrière", "Reducir"), action: #selector(zoomOut), keyEquivalent: "-")
        zoomOut.target = self
        let zoomReset = viewMenu.addItem(withTitle: L("Originalgröße", "Actual Size", "Taille réelle", "Tamaño real"), action: #selector(zoomReset), keyEquivalent: "0")
        zoomReset.target = self
        viewMenu.addItem(.separator())
        viewMenu.addItem(withTitle: L("Vollbild", "Enter Full Screen", "Passer en plein écran", "Entrar en pantalla completa"), action: #selector(NSWindow.toggleFullScreen(_:)), keyEquivalent: "f")
            .keyEquivalentModifierMask = [.command, .control]
        viewItem.submenu = viewMenu
        main.addItem(viewItem)

        let windowItem = NSMenuItem()
        let windowMenu = NSMenu(title: L("Fenster", "Window", "Fenêtre", "Ventana"))
        windowMenu.addItem(withTitle: L("Im Dock ablegen", "Minimize", "Placer dans le Dock", "Minimizar"), action: #selector(NSWindow.performMiniaturize(_:)), keyEquivalent: "m")
        windowMenu.addItem(withTitle: L("Schließen", "Close", "Fermer", "Cerrar"), action: #selector(NSWindow.performClose(_:)), keyEquivalent: "w")
        windowItem.submenu = windowMenu
        main.addItem(windowItem)
        NSApp.windowsMenu = windowMenu

        let helpItem = NSMenuItem()
        let helpMenu = NSMenu(title: L("Hilfe", "Help", "Aide", "Ayuda"))
        let log = helpMenu.addItem(withTitle: L("Protokoll im Finder zeigen", "Show Log in Finder", "Afficher le journal dans le Finder", "Mostrar el registro en el Finder"), action: #selector(showLogFromMenu), keyEquivalent: "")
        log.target = self
        let releases = helpMenu.addItem(withTitle: L("Releases auf GitHub", "Releases on GitHub", "Versions sur GitHub", "Versiones en GitHub"), action: #selector(openReleases), keyEquivalent: "")
        releases.target = self
        helpItem.submenu = helpMenu
        main.addItem(helpItem)
        NSApp.helpMenu = helpMenu

        NSApp.mainMenu = main
    }

    @objc private func reloadInterface() { webView.reload() }
    @objc private func zoomIn() { webView.pageZoom = min(webView.pageZoom + 0.1, 3) }
    @objc private func zoomOut() { webView.pageZoom = max(webView.pageZoom - 0.1, 0.5) }
    @objc private func zoomReset() { webView.pageZoom = 1 }
    @objc private func showLogFromMenu() { showLog() }
    @objc private func openReleases() { NSWorkspace.shared.open(Self.releasesPage) }

    private func showLog() {
        NSWorkspace.shared.activateFileViewerSelecting([WorkerController.logURL])
    }

    // MARK: Dialog helpers

    private func inform(_ message: String) {
        let alert = NSAlert()
        alert.messageText = message
        alert.addButton(withTitle: "OK")
        alert.runModal()
    }

    private func confirm(_ message: String, ok: String, cancel: String) -> Bool {
        let alert = NSAlert()
        alert.messageText = message
        alert.addButton(withTitle: ok)
        alert.addButton(withTitle: cancel)
        return alert.runModal() == .alertFirstButtonReturn
    }

    // MARK: WKNavigationDelegate

    /// Only the local interface may load in this window; everything else opens in the default browser.
    func webView(_ webView: WKWebView, decidePolicyFor navigationAction: WKNavigationAction) async -> WKNavigationActionPolicy {
        guard let url = navigationAction.request.url else { return .cancel }
        if url.scheme == "about" { return .allow }
        if (url.host == "127.0.0.1" || url.host == "localhost") && url.port == worker.port { return .allow }
        if url.scheme == "http" || url.scheme == "https" || url.scheme == "mailto" {
            NSWorkspace.shared.open(url)
        }
        return .cancel
    }

    // MARK: WKUIDelegate - the web interface uses confirm() and prompt(), which WKWebView ignores unless handled here

    func webView(_ webView: WKWebView, createWebViewWith configuration: WKWebViewConfiguration,
                 for navigationAction: WKNavigationAction, windowFeatures: WKWindowFeatures) -> WKWebView? {
        if let url = navigationAction.request.url { NSWorkspace.shared.open(url) }
        return nil
    }

    func webView(_ webView: WKWebView, runJavaScriptAlertPanelWithMessage message: String,
                 initiatedByFrame frame: WKFrameInfo) async {
        inform(message)
    }

    func webView(_ webView: WKWebView, runJavaScriptConfirmPanelWithMessage message: String,
                 initiatedByFrame frame: WKFrameInfo) async -> Bool {
        confirm(message, ok: "OK", cancel: L("Abbrechen", "Cancel", "Annuler", "Cancelar"))
    }

    func webView(_ webView: WKWebView, runJavaScriptTextInputPanelWithPrompt prompt: String,
                 defaultText: String?, initiatedByFrame frame: WKFrameInfo) async -> String? {
        let alert = NSAlert()
        alert.messageText = prompt
        let field = NSTextField(frame: NSRect(x: 0, y: 0, width: 320, height: 24))
        field.stringValue = defaultText ?? ""
        alert.accessoryView = field
        alert.addButton(withTitle: "OK")
        alert.addButton(withTitle: L("Abbrechen", "Cancel", "Annuler", "Cancelar"))
        alert.window.initialFirstResponder = field
        return alert.runModal() == .alertFirstButtonReturn ? field.stringValue : nil
    }

    func webView(_ webView: WKWebView, runOpenPanelWith parameters: WKOpenPanelParameters,
                 initiatedByFrame frame: WKFrameInfo) async -> [URL]? {
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = parameters.allowsMultipleSelection
        panel.canChooseDirectories = parameters.allowsDirectories
        return panel.runModal() == .OK ? panel.urls : nil
    }
}
