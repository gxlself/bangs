import UIKit
import UserNotifications

/// Registers for remote notifications, so the CloudKit zone subscription (silent push) can
/// wake the app, and pulls when one arrives. Also shows the "Claude is waiting" notifications
/// while the app is open, and opens the Dev tab when one is tapped.
final class AppDelegate: NSObject, UIApplicationDelegate, UNUserNotificationCenterDelegate {

    func application(
        _ application: UIApplication,
        didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]? = nil
    ) -> Bool {
        UNUserNotificationCenter.current().delegate = self
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

    // MARK: UNUserNotificationCenterDelegate

    /// In the foreground too: the Dev tab may not be the one on screen.
    nonisolated func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        willPresent notification: UNNotification,
        withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void
    ) {
        completionHandler([.banner, .list, .sound])
    }

    nonisolated func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        didReceive response: UNNotificationResponse,
        withCompletionHandler completionHandler: @escaping () -> Void
    ) {
        let opensDev = (response.notification.request.content.userInfo["tab"] as? String) == "dev"
        if opensDev {
            Task { @MainActor in
                SyncModel.shared.selectedTab = .dev
            }
        }
        completionHandler()
    }
}
