// iCloud bridge for the watch API (src/watch.rs).
//
// While watch access is on, Bangs keeps one record in the private CloudKit
// database of the Apple ID signed in on this Mac: its name, its addresses on
// the local network and a token for the watch API. A watch signed in to the
// same Apple ID reads it and connects without a pairing code. Nobody else can
// read a private database, and the payload is an encrypted field on top.
//
// CloudKit throws if the process lacks the container entitlement, which every
// build without the iCloud provisioning profile does (`pnpm tauri dev`, the
// plain release build), so every call checks the entitlement first and does
// nothing without it.

#import <CloudKit/CloudKit.h>
#import <Foundation/Foundation.h>
#import <Security/Security.h>

static NSString *const kZoneName = @"BangsWatch";
static NSString *const kRecordType = @"BangsMac";
static NSString *const kPayloadKey = @"payload";

static BOOL HasContainer(NSString *container) {
    SecTaskRef task = SecTaskCreateFromSelf(NULL);
    if (!task) return NO;
    CFTypeRef value = SecTaskCopyValueForEntitlement(
        task, CFSTR("com.apple.developer.icloud-container-identifiers"), NULL);
    CFRelease(task);
    if (!value) return NO;
    BOOL found = CFGetTypeID(value) == CFArrayGetTypeID() &&
                 [(__bridge NSArray *)value containsObject:container];
    CFRelease(value);
    return found;
}

static CKRecordZoneID *ZoneID(void) {
    return [[CKRecordZoneID alloc] initWithZoneName:kZoneName ownerName:CKCurrentUserDefaultName];
}

int bangs_icloud_available(const char *container) {
    @autoreleasepool {
        return HasContainer(@(container)) ? 1 : 0;
    }
}

// Creates the zone when it is missing and writes the record over whatever was
// there. Returns at once; failures are logged.
void bangs_icloud_publish(const char *container, const char *record_name, const char *payload) {
    @autoreleasepool {
        NSString *identifier = @(container);
        if (!HasContainer(identifier)) return;
        CKDatabase *database = [[CKContainer containerWithIdentifier:identifier] privateCloudDatabase];
        CKRecordZoneID *zoneID = ZoneID();

        CKRecordZone *zone = [[CKRecordZone alloc] initWithZoneID:zoneID];
        CKModifyRecordZonesOperation *zones =
            [[CKModifyRecordZonesOperation alloc] initWithRecordZonesToSave:@[ zone ] recordZoneIDsToDelete:nil];
        zones.modifyRecordZonesResultBlock = ^(NSError *error) {
          if (error) NSLog(@"[watch] iCloud zone: %@", error);
        };

        CKRecordID *recordID = [[CKRecordID alloc] initWithRecordName:@(record_name) zoneID:zoneID];
        CKRecord *record = [[CKRecord alloc] initWithRecordType:kRecordType recordID:recordID];
        record.encryptedValues[kPayloadKey] = @(payload);
        CKModifyRecordsOperation *save =
            [[CKModifyRecordsOperation alloc] initWithRecordsToSave:@[ record ] recordIDsToDelete:nil];
        // Overwrite without fetching first: this Mac is the only writer.
        save.savePolicy = CKRecordSaveAllKeys;
        save.modifyRecordsResultBlock = ^(NSError *error) {
          if (error) NSLog(@"[watch] iCloud publish: %@", error);
        };
        [save addDependency:zones];
        [database addOperation:zones];
        [database addOperation:save];
    }
}

// Takes this Mac's record down, when watch access is turned off.
void bangs_icloud_remove(const char *container, const char *record_name) {
    @autoreleasepool {
        NSString *identifier = @(container);
        if (!HasContainer(identifier)) return;
        CKDatabase *database = [[CKContainer containerWithIdentifier:identifier] privateCloudDatabase];
        CKRecordID *recordID = [[CKRecordID alloc] initWithRecordName:@(record_name) zoneID:ZoneID()];
        CKModifyRecordsOperation *remove =
            [[CKModifyRecordsOperation alloc] initWithRecordsToSave:nil recordIDsToDelete:@[ recordID ]];
        remove.modifyRecordsResultBlock = ^(NSError *error) {
          if (error) NSLog(@"[watch] iCloud remove: %@", error);
        };
        [database addOperation:remove];
    }
}
