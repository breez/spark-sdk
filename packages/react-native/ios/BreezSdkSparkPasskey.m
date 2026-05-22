#import <React/RCTBridgeModule.h>

@interface RCT_EXTERN_MODULE(BreezSdkSparkPasskey, NSObject)

RCT_EXTERN_METHOD(deriveSeeds:(NSArray *)salts
                  rpId:(NSString *)rpId
                  rpName:(NSString *)rpName
                  userName:(NSString *)userName
                  userDisplayName:(NSString *)userDisplayName
                  autoRegister:(BOOL)autoRegister
                  allowCredentials:(NSArray *)allowCredentials
                  preferImmediatelyAvailableCredentials:(nullable NSNumber *)preferImmediatelyAvailableCredentials
                  resolve:(RCTPromiseResolveBlock)resolve
                  reject:(RCTPromiseRejectBlock)reject)

RCT_EXTERN_METHOD(createPasskey:(NSString *)rpId
                  rpName:(NSString *)rpName
                  userName:(NSString *)userName
                  userDisplayName:(NSString *)userDisplayName
                  excludeCredentials:(NSArray *)excludeCredentials
                  resolve:(RCTPromiseResolveBlock)resolve
                  reject:(RCTPromiseRejectBlock)reject)

RCT_EXTERN_METHOD(isSupported:(RCTPromiseResolveBlock)resolve
                  reject:(RCTPromiseRejectBlock)reject)

RCT_EXTERN_METHOD(checkDomainAssociation:(NSString *)rpId
                  teamId:(NSString *)teamId
                  resolve:(RCTPromiseResolveBlock)resolve
                  reject:(RCTPromiseRejectBlock)reject)

@end
