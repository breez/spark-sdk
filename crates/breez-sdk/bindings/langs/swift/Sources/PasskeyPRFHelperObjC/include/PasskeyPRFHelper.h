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

/// Set PRF assertion input (with up to two salts) on an assertion request.
/// Pass nil for `salt2` to skip the second derivation.
+ (void)setAssertionPRFOnRequest:(ASAuthorizationPlatformPublicKeyCredentialAssertionRequest *)request
                       withSalt1:(NSData *)salt1
                           salt2:(NSData * _Nullable)salt2;

/// Set PRF registration input on a registration request.
+ (void)setRegistrationPRFOnRequest:(ASAuthorizationPlatformPublicKeyCredentialRegistrationRequest *)request;

/// Extract the PRF output (first salt result) from an assertion credential.
/// Returns nil if PRF output is not available.
+ (NSData * _Nullable)extractPRFOutputFromAssertion:(ASAuthorizationPlatformPublicKeyCredentialAssertion *)credential;

/// Extract the second PRF output (second salt result) from an assertion
/// credential. Returns nil if PRF output is not available, or if no
/// second salt was provided in the request.
+ (NSData * _Nullable)extractSecondPRFOutputFromAssertion:(ASAuthorizationPlatformPublicKeyCredentialAssertion *)credential;

@end

NS_ASSUME_NONNULL_END
