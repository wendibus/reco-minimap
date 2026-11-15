import Foundation

/// The app's own texts (menus, dialogs) in the language of the system: German, English, French or
/// Spanish, English for anything else. The interface itself (web) has its own language switch.
func L(_ german: String, _ english: String, _ french: String, _ spanish: String) -> String {
    let lang = String((Locale.preferredLanguages.first ?? "en").prefix(2))
    switch lang {
    case "de": return german
    case "fr": return french
    case "es": return spanish
    default: return english
    }
}
