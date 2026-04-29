#import <Foundation/Foundation.h>
#import <AuthenticationServices/AuthenticationServices.h>

NS_ASSUME_NONNULL_BEGIN

/// ObjC helper for passkey PRF operations.
///
/// The PRF types are marked NS_REFINED_FOR_SWIFT in the iOS/macOS SDK, which makes
/// their initializers inaccessible from Swift. This helper bridges the gap.
API_AVAILABLE(ios(18.0), macos(15.0))
@interface PasskeyPRFHelper : NSObject

/// Set PRF assertion input (with salt) on an assertion request.
+ (void)setAssertionPRFOnRequest:(ASAuthorizationPlatformPublicKeyCredentialAssertionRequest *)request
                        withSalt:(NSData *)salt;

/// Set PRF registration input on a registration request.
+ (void)setRegistrationPRFOnRequest:(ASAuthorizationPlatformPublicKeyCredentialRegistrationRequest *)request;

/// Extract the PRF output (first salt result) from an assertion credential.
/// Returns nil if PRF output is not available.
+ (NSData * _Nullable)extractPRFOutputFromAssertion:(ASAuthorizationPlatformPublicKeyCredentialAssertion *)credential;

@end

NS_ASSUME_NONNULL_END
