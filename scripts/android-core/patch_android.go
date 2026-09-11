//go:build android && cmfa

package route

// Mfork launches a standalone executable, not CMFA's embedded library.
// Keep CMFA's non-root TUN integration but expose the authenticated
// controller configuration routes used by the application.
func init() {
	SetEmbedMode(false)
}
