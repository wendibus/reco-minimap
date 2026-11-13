/// Public Ed25519 key (base64, 32 raw bytes) that update payloads of Reco Minimap must be signed with.
/// Generate the key pair once with `swift mac/tools/payload-signing.swift keygen` (the private key goes
/// to ~/.reco-minimap/payload-signing.key, never into the repository) and paste the printed public key
/// here. While this is empty the launcher never installs downloaded updates - it only runs the payload
/// bundled inside the app.
public enum PayloadKey {
    public static let publicKeyBase64 = "3R4Ee+/MZ91QXpFhYl0szpC8iWfrGrM63hvpuCJXAEs="
}
