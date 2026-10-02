import UIKit

/// Registers for remote notifications, so the CloudKit zone subscription (silent push) can
/// wake the app, and pulls when one arrives.
final class AppDelegate: NSObject, UIApplicationDelegate {

    func application(
        _ application: UIApplication,
        didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]? = nil
    ) -> Bool {
        application.registerForRemoteNotifications()
        return true
    }

    func application(
        _ application: UIApplication,
        didReceiveRemoteNotification userInfo: [AnyHashable: Any],
        fetchCompletionHandler completionHandler: @escaping (UIBackgroundFetchResult) -> Void
    ) {
        Task { @MainActor in
            await SyncModel.shared.handleRemoteNotification()
            completionHandler(.newData)
        }
    }

    func application(_ application: UIApplication, didFailToRegisterForRemoteNotificationsWithError error: Error) {
        // Normal on the Simulator, or when the provisioning profile has no Push Notifications.
        // Sync still works while the app is open (it pulls on launch, on foreground and every minute).
        print("AppDelegate: failed to register for remote notifications: \(error.localizedDescription)")
    }
}
