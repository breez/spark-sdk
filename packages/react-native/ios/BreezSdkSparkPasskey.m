#import <React/RCTBridgeModule.h>

@interface RCT_EXTERN_MODULE(BreezSdkSparkPasskey, NSObject)

RCT_EXTERN_METHOD(derivePrfSeed:(NSString *)salt
                  rpId:(NSString *)rpId
                  rpName:(NSString *)rpName
                  userName:(NSString *)userName
                  userDisplayName:(NSString *)userDisplayName
                  autoRegister:(BOOL)autoRegister
                  resolve:(RCTPromiseResolveBlock)resolve
                  reject:(RCTPromiseRejectBlock)reject)

RCT_EXTERN_METHOD(createPasskey:(NSString *)rpId
                  rpName:(NSString *)rpName
                  userName:(NSString *)userName
                  userDisplayName:(NSString *)userDisplayName
                  excludeCredentialIds:(NSArray *)excludeCredentialIds
                  resolve:(RCTPromiseResolveBlock)resolve
                  reject:(RCTPromiseRejectBlock)reject)

RCT_EXTERN_METHOD(isPrfAvailable:(RCTPromiseResolveBlock)resolve
                  reject:(RCTPromiseRejectBlock)reject)

@end
