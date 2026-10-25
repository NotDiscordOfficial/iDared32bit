/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */

#ifndef TOUCHHLE_SYSTEM_HEADERS_H
#define TOUCHHLE_SYSTEM_HEADERS_H

// This file contains definitions of types etc we don't have in our SDK, which
// is built from open-source headers.

#include <CoreFoundation/CFBundle.h>
#include <CoreFoundation/CFData.h>
#include <CoreFoundation/CFDate.h>
#include <CoreFoundation/CFRunLoop.h>
#include <CoreFoundation/CFURL.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

// CommonCrypto

#define CC_SHA256_DIGEST_LENGTH 32
typedef uint32_t CC_LONG;
typedef struct CC_SHA256state_st {
  CC_LONG count[2];
  CC_LONG hash[8];
  CC_LONG wbuf[16];
} CC_SHA256_CTX;
unsigned char *CC_SHA256(const void *, CC_LONG, unsigned char *);
int CC_SHA256_Init(CC_SHA256_CTX *);
int CC_SHA256_Update(CC_SHA256_CTX *, const void *, CC_LONG);
int CC_SHA256_Final(unsigned char *, CC_SHA256_CTX *);

// Objective-C runtime

typedef signed char BOOL;
#define YES 1
#define NO 0

typedef unsigned long NSUInteger;
typedef signed long NSInteger;

typedef struct _NSRange {
  NSUInteger location;
  NSUInteger length;
} NSRange;

static inline NSRange NSMakeRange(NSUInteger loc, NSUInteger len) {
  NSRange r;
  r.location = loc;
  r.length = len;
  return r;
}

#define nil ((id)0)

// id objc_msgSend(id, SEL, ...);

typedef struct objc_method *Method;
Method class_getInstanceMethod(Class cls, SEL name);
Method class_getClassMethod(Class cls, SEL name);
typedef void (*TestMethodIMP)(void);
TestMethodIMP method_setImplementation(Method m, TestMethodIMP imp);
#ifdef DEFINE_ME_WHEN_BUILDING_ON_MACOS
SEL method_getName(Method m);
// Returns IMP, which isn't declared here.
void (*method_getImplementation(Method m))(void);
#endif

// Foundation

@interface NSObject {
  Class isa;
}
+ (Class)class;
+ (instancetype)alloc;
+ (instancetype)new;
+ (BOOL)respondsToSelector:(SEL)selector;
+ (id)instanceMethodSignatureForSelector:(SEL)selector;
- (instancetype)init;
- (instancetype)retain;
- (void)release;
- (instancetype)autorelease;
- (void)dealloc;
- (NSUInteger)retainCount;
- (id)performSelector:(SEL)selector;
- (BOOL)respondsToSelector:(SEL)selector;
- (id)methodSignatureForSelector:(SEL)selector;
- (void)addObserver:(id)observer
         forKeyPath:(NSString *)keyPath
            options:(NSUInteger)options
            context:(void *)context;
- (void)removeObserver:(id)observer forKeyPath:(NSString *)keyPath;
- (void)removeObserver:(id)observer
            forKeyPath:(NSString *)keyPath
               context:(void *)context;
- (void)performSelector:(SEL)selector
               onThread:(id)thread
             withObject:(id)object
          waitUntilDone:(BOOL)wait
                  modes:(id)modes;
@end

@interface NSAutoreleasePool : NSObject
+ (void)addObject:(id)anObject;
- (void)addObject:(id)anObject;
- (void)drain;
@end

@interface NSArray<ObjectType> : NSObject
+ (instancetype)array;
+ (instancetype)arrayWithObjects:(ObjectType)firstObj, ...;
- (NSUInteger)count;
- (ObjectType)objectAtIndex:(NSUInteger)index;
- (BOOL)isEqualToArray:(NSArray *)otherArray;
@end

@interface NSDictionary<KeyType, ObjectType> : NSObject
+ (instancetype)dictionaryWithObjects:(NSArray<ObjectType> *)objects
                              forKeys:(NSArray<KeyType> *)keys;
- (NSUInteger)count;
- (ObjectType)objectForKey:(KeyType)aKey;
- (BOOL)isEqualToDictionary:(NSDictionary *)otherDictionary;
@end

@interface NSSet<ObjectType> : NSObject
 (instancetype)setWithObjects:(ObjectType)firstObject, ...;
- (NSUInteger)count;
- (BOOL)containsObject:(ObjectType)object;
- (id)mutableCopyWithZone:(void *)zone;
- (ObjectType)anyObject;
@end

@interface NSMutableSet<ObjectType> : NSSet<ObjectType>
- (void)addObject:(ObjectType)object;
- (id)copyWithZone:(void *)zone;
@end

typedef enum {
  NSCaseInsensitiveSearch = 1,
} NSStringCompareOptions;

@interface NSString : NSObject
+ (instancetype)stringWithFormat:(NSString *)format, ...;
+ (instancetype)stringWithUTF8String:(const char *)string;
+ (NSString *)pathWithComponents:(NSArray *)components;
- (NSString *)stringByReplacingOccurrencesOfString:(NSString *)target
                                        withString:(NSString *)replacement;
- (NSString *)stringByReplacingOccurrencesOfString:(NSString *)target
                                        withString:(NSString *)replacement
                                           options:
                                               (NSStringCompareOptions)options
                                             range:(NSRange)range;
- (BOOL)isEqualToString:(NSString *)other;
@end
@interface NSMutableString : NSString
- (void)deleteCharactersInRange:(NSRange)range;
@end

@interface NSValue : NSObject
@end

@interface NSNumber : NSValue
+ (NSNumber *)numberWithFloat:(float)value;
+ (NSNumber *)numberWithBool:(bool)value;
@end

NSString *NSStringFromClass(Class);

typedef double NSTimeInterval;

@interface NSProcessInfo : NSObject
+ (instancetype)processInfo;
- (NSTimeInterval)systemUptime;
@end

@interface NSTimer : NSObject
+ (instancetype)timerWithTimeInterval:(NSTimeInterval)interval
                               target:(id)target
                             selector:(SEL)selector
                             userInfo:(id)user_info
                              repeats:(BOOL)repeats;
+ (instancetype)scheduledTimerWithTimeInterval:(NSTimeInterval)interval
                                        target:(id)target
                                      selector:(SEL)selector
                                      userInfo:(id)user_info
                                       repeats:(BOOL)repeats;
- (void)invalidate;
@end

@class NSData;

@interface NSURL : NSObject
+ (instancetype)URLWithString:(NSString *)URLString;
@end

@interface NSURLRequest : NSObject
- (instancetype)initWithURL:(NSURL *)url;
- (NSURL *)URL;
- (NSString *)HTTPMethod;
- (NSDictionary *)allHTTPHeaderFields;
- (NSTimeInterval)timeoutInterval;
@end

@interface NSMutableURLRequest : NSURLRequest
- (void)setTimeoutInterval:(NSTimeInterval)timeoutInterval;
- (void)setHTTPMethod:(NSString *)method;
- (void)setHTTPBody:(NSData *)body;
- (void)setValue:(NSString *)value forHTTPHeaderField:(NSString *)field;
@end

@interface NSData : NSObject
+ (id)dataWithContentsOfURL:(NSURL *)url;
@end

@interface NSCoder : NSObject
- (void)encodeBytes:(const uint8_t *)bytes
             length:(NSUInteger)length
             forKey:(NSString *)key;
- (const uint8_t *)decodeBytesForKey:(NSString *)key
                      returnedLength:(NSUInteger *)lengthp;
- (void)encodeInt:(int)value forKey:(NSString *)key;
- (int)decodeIntForKey:(NSString *)key;
@end

@interface NSKeyedArchiver : NSCoder
+ (NSData *)archivedDataWithRootObject:(id)rootObject;
@end

@interface NSKeyedUnarchiver : NSCoder
+ (id)unarchiveObjectWithData:(NSData *)data;
@end

SEL NSSelectorFromString(NSString *);

@interface NSMethodSignature : NSObject
+ (instancetype)signatureWithObjCTypes:(const char *)types;
- (NSUInteger)numberOfArguments;
- (const char *)getArgumentTypeAtIndex:(NSUInteger)idx;
- (const char *)methodReturnType;
@end

@interface NSInvocation : NSObject
+ (instancetype)invocationWithMethodSignature:(NSMethodSignature *)sig;
- (void)setTarget:(id)target;
- (void)setSelector:(SEL)sel;
- (void)setArgument:(void *)arg atIndex:(NSInteger)idx;
- (void)retainArguments;
- (void)invoke;
- (void)invokeWithTarget:(id)target;
@end

@interface NSOperation : NSObject
- (void)start;
- (BOOL)isFinished;
@end

@interface NSInvocationOperation : NSOperation
- (instancetype)initWithTarget:(id)target
                      selector:(SEL)selector
                        object:(id)object;
- (instancetype)initWithInvocation:(NSInvocation *)invocation;
@end

@interface NSNotification : NSObject
- (NSString *)name;
- (id)object;
- (NSDictionary *)userInfo;
@end

@interface NSNotificationCenter : NSObject
+ (NSNotificationCenter *)defaultCenter;
- (void)addObserver:(id)observer
           selector:(SEL)selector
               name:(NSString *)name
             object:(id)object;
- (void)removeObserver:(id)observer;
- (void)removeObserver:(id)observer name:(NSString *)name object:(id)object;
- (void)postNotificationName:(NSString *)name object:(id)object;
- (void)postNotificationName:(NSString *)name
                      object:(id)object
                    userInfo:(NSDictionary *)userInfo;
@end

@interface NSConditionLock : NSObject
- (instancetype)initWithCondition:(NSInteger)condition;
- (NSInteger)condition;
- (void)lock;
- (void)unlock;
- (BOOL)tryLock;
- (void)lockWhenCondition:(NSInteger)condition;
- (BOOL)tryLockWhenCondition:(NSInteger)condition;
- (void)unlockWithCondition:(NSInteger)condition;
@end

@interface NSThread : NSObject
+ (NSThread *)currentThread;
+ (void)exit;
- (instancetype)initWithTarget:(id)target
                      selector:(SEL)selector
                        object:(id)argument;
- (void)start;
- (BOOL)isExecuting;
- (BOOL)isFinished;
@end

// Core Graphics

// (See CGAffineTransform.c for where this define comes from.)
#ifdef DEFINE_ME_WHEN_BUILDING_ON_MACOS
typedef double CGFloat; // 64-bit definition (not supported by touchHLE)
#else
typedef float CGFloat;
#endif

typedef struct {
  CGFloat x, y;
} CGPoint;
bool CGPointEqualToPoint(CGPoint, CGPoint);
static inline CGPoint CGPointMake(CGFloat x, CGFloat y) {
  return (CGPoint){x, y};
}
typedef struct {
  CGFloat width, height;
} CGSize;
bool CGSizeEqualToSize(CGSize, CGSize);
static inline CGSize CGSizeMake(CGFloat width, CGFloat height) {
  return (CGSize){width, height};
}
typedef struct {
  CGPoint origin;
  CGSize size;
} CGRect;
bool CGRectEqualToRect(CGRect, CGRect);
static inline CGRect CGRectMake(CGFloat x, CGFloat y, CGFloat width,
                                CGFloat height) {
  return (CGRect){CGPointMake(x, y), CGSizeMake(width, height)};
}

typedef struct {
  CGFloat a, b, c, d, tx, ty;
} CGAffineTransform;
extern const CGAffineTransform CGAffineTransformIdentity;
bool CGAffineTransformIsIdentity(CGAffineTransform);
bool CGAffineTransformEqualToTransform(CGAffineTransform, CGAffineTransform);
CGAffineTransform CGAffineTransformMake(CGFloat, CGFloat, CGFloat, CGFloat,
                                        CGFloat, CGFloat);
CGAffineTransform CGAffineTransformMakeRotation(CGFloat);
CGAffineTransform CGAffineTransformMakeScale(CGFloat, CGFloat);
CGAffineTransform CGAffineTransformMakeTranslation(CGFloat, CGFloat);
CGAffineTransform CGAffineTransformConcat(CGAffineTransform, CGAffineTransform);
CGAffineTransform CGAffineTransformRotate(CGAffineTransform, CGFloat);
CGAffineTransform CGAffineTransformScale(CGAffineTransform, CGFloat, CGFloat);
CGAffineTransform CGAffineTransformTranslate(CGAffineTransform, CGFloat,
                                             CGFloat);
CGAffineTransform CGAffineTransformInvert(CGAffineTransform);
CGPoint CGPointApplyAffineTransform(CGPoint, CGAffineTransform);
CGSize CGSizeApplyAffineTransform(CGSize, CGAffineTransform);
CGRect CGRectApplyAffineTransform(CGRect, CGAffineTransform);

@interface NSValue (CGGeometryNSValueAdditions)
+ (instancetype)valueWithCGPoint:(CGPoint)point;
+ (instancetype)valueWithCGRect:(CGRect)rect;
@end

// `CGDataProvider.h`

typedef struct _CGDataProvider *CGDataProviderRef;

CGDataProviderRef CGDataProviderCreateWithCFData(CFDataRef);
CFDataRef CGDataProviderCopyData(CGDataProviderRef);

// `CGGeometry.h`

CGFloat CGRectGetMinX(CGRect);
CGFloat CGRectGetMaxX(CGRect);
CGFloat CGRectGetMinY(CGRect);
CGFloat CGRectGetMaxY(CGRect);
CGFloat CGRectGetHeight(CGRect);
CGFloat CGRectGetWidth(CGRect);

// `CGImage.h`

typedef struct _CGImage *CGImageRef;

CGImageRef CGImageCreateWithJPEGDataProvider(CGDataProviderRef, const CGFloat *,
                                             bool, int);
size_t CGImageGetWidth(CGImageRef);
size_t CGImageGetHeight(CGImageRef);
CGDataProviderRef CGImageGetDataProvider(CGImageRef);
void CGImageRelease(CGImageRef image);

// `CGColor.h`

typedef struct _CGColor *CGColorRef;

CGColorRef CGColorCreateGenericRGB(CGFloat red, CGFloat green, CGFloat blue,
                                   CGFloat alpha);

// `CGColorSpace.h`

typedef struct _CGColorSpace *CGColorSpaceRef;

CGColorSpaceRef CGColorSpaceCreateDeviceRGB(void);
void CGColorSpaceRelease(CGColorSpaceRef cs);

// `CGContext.h`

typedef struct _CGContext *CGContextRef;

typedef enum {
  kCGBlendModeNormal = 0,
  kCGBlendModeMultiply = 1,
  kCGBlendModeScreen = 2,
  kCGBlendModeOverlay = 3,
  kCGBlendModeDarken = 4,
  kCGBlendModeLighten = 5,
} CGBlendMode;

#define kCGImageAlphaPremultipliedLast 1

CGContextRef CGBitmapContextCreate(void *data, size_t width, size_t height,
                                   size_t bitsPerComponent, size_t bytesPerRow,
                                   CGColorSpaceRef space,
                                   unsigned int bitmapInfo);
CGImageRef CGBitmapContextCreateImage(CGContextRef c);
void CGContextRelease(CGContextRef c);
void CGContextSaveGState(CGContextRef c);
void CGContextRestoreGState(CGContextRef c);
void CGContextSetRGBFillColor(CGContextRef c, CGFloat r, CGFloat g, CGFloat b,
                              CGFloat a);
void CGContextFillRect(CGContextRef c, CGRect rect);
void CGContextSetBlendMode(CGContextRef c, CGBlendMode mode);
void CGContextTranslateCTM(CGContextRef c, CGFloat tx, CGFloat ty);
void CGContextScaleCTM(CGContextRef c, CGFloat sx, CGFloat sy);
void CGContextRotateCTM(CGContextRef c, CGFloat angle);

// `CGFont.h` and `CGContext.h` text functions.

typedef struct _CGFont *CGFontRef;
typedef unsigned short CGGlyph;

CGFontRef CGFontCreateWithDataProvider(CGDataProviderRef name);
void CGFontRelease(CGFontRef font);

void CGContextSetFont(CGContextRef c, CGFontRef font);
void CGContextSetFontSize(CGContextRef c, CGFloat size);
void CGContextShowGlyphsAtPoint(CGContextRef c, CGFloat x, CGFloat y,
                                const CGGlyph *glyphs, size_t count);

// `UIGraphics.h`

CGContextRef UIGraphicsGetCurrentContext(void);

// AudioToolbox

typedef struct AudioStreamBasicDescription {
  double mSampleRate;
  uint32_t mFormatID;
  uint32_t mFormatFlags;
  uint32_t mBytesPerPacket;
  uint32_t mFramesPerPacket;
  uint32_t mBytesPerFrame;
  uint32_t mChannelsPerFrame;
  uint32_t mBitsPerChannel;
  uint32_t mReserved;
} AudioStreamBasicDescription;
typedef struct SMPTETime {
  int16_t mSubframes;
  int16_t mSubframeDivisor;
  uint32_t mCounter;
  uint32_t mType;
  uint32_t mFlags;
  int16_t mHours;
  int16_t mMinutes;
  int16_t mSeconds;
  int16_t mFrames;
} SMPTETime;
typedef struct AudioTimeStamp {
  double mSampleTime;
  uint64_t mHostTime;
  double mRateScalar;
  uint64_t mWordClockTime;
  SMPTETime mSMPTETime;
  uint32_t mFlags;
  uint32_t mReserved;
} AudioTimeStamp;
#define kAudioFormatLinearPCM 'lpcm'
#define kAudioFormatFlagIsSignedInteger (1 << 2)
#define kAudioFormatFlagIsPacked (1 << 3)
#define kAudioQueueErr_InvalidRunState -66678
typedef struct OpaqueAudioQueue *AudioQueueRef;
typedef struct OpaqueAudioQueueTimeline *AudioQueueTimelineRef;
typedef struct AudioQueueBuffer *AudioQueueBufferRef;
typedef void (*AudioQueueOutputCallback)(void *, AudioQueueRef,
                                         AudioQueueBufferRef);
OSStatus AudioQueueNewOutput(const AudioStreamBasicDescription *,
                             AudioQueueOutputCallback, void *, CFRunLoopRef,
                             CFStringRef, uint32_t, AudioQueueRef *);
OSStatus AudioQueueDispose(AudioQueueRef, Boolean);
OSStatus AudioQueueCreateTimeline(AudioQueueRef, AudioQueueTimelineRef *);
OSStatus AudioQueueDisposeTimeline(AudioQueueRef, AudioQueueTimelineRef);
OSStatus AudioQueueGetCurrentTime(AudioQueueRef, AudioQueueTimelineRef,
                                  AudioTimeStamp *, Boolean *);

// Core Animation
typedef NSString *CAMediaTimingFunctionName;

CFTimeInterval CACurrentMediaTime();

@interface CAMediaTimingFunction : NSObject
+ (instancetype)functionWithName:(CAMediaTimingFunctionName)name;
@end
@interface CAAnimation : NSObject
- (void)setTimingFunction:(CAMediaTimingFunction *)timingFunction;
- (CFTimeInterval)duration;
- (void)setDuration:(CFTimeInterval)duration;
- (void)setBeginTime:(CFTimeInterval)beginTime;
- (void)setRepeatCount:(float)repeatCount;
- (void)setAutoreverses:(bool)autoreverses;
@end
@interface CAPropertyAnimation : CAAnimation
+ (instancetype)animationWithKeyPath:(NSString *)path;
@end
@interface CABasicAnimation : CAPropertyAnimation
- (void)setFromValue:(id)value;
- (void)setToValue:(id)value;
@end
@interface CALayer : NSObject
- (void)setAffineTransform:(CGAffineTransform)transform;
- (void)setAnchorPoint:(CGPoint)point;
- (void)setCornerRadius:(CGFloat)radius;
- (CGPoint)position;
- (void)setPosition:(CGPoint)position;
- (CGRect)bounds;
- (void)setBounds:(CGRect)bounds;
- (CGRect)frame;
- (void)setFrame:(CGRect)frame;
- (CGColorRef)backgroundColor;
- (void)setBackgroundColor:(CGColorRef)newColorRef;
- (CGPoint)convertPoint:(CGPoint)point fromLayer:(CALayer *)layer;
- (CGPoint)convertPoint:(CGPoint)point toLayer:(CALayer *)layer;
- (CGRect)convertRect:(CGRect)point fromLayer:(CALayer *)layer;
- (CGRect)convertRect:(CGRect)point toLayer:(CALayer *)layer;
- (void)addAnimation:(CAAnimation *)anim forKey:(NSString *)key;
- (void)removeAnimationForKey:(NSString *)key;
@end
@interface CATransaction : NSObject
+ (void)setValue:(id)value forKey:(NSString *)key;
+ (id)valueForKey:(NSString *)key;
+ (void)begin;
+ (void)commit;
+ (bool)disableActions;
+ (void)setDisableActions:(bool)flag;
+ (CFTimeInterval)animationDuration;
+ (void)setAnimationDuration:(CFTimeInterval)duration;
+ (id)animationTimingFunction;
+ (void)setAnimationTimingFunction:
    (CAMediaTimingFunction *)animation_timing_function;
@end

// UIKit

typedef enum {
  UITextAlignmentLeft = 0,
  UITextAlignmentCenter = 1,
  UITextAlignmentRight = 2,
} UITextAlignment;

typedef enum {
  UIButtonTypeRoundedRect = 1,
} UIButtonType;

typedef enum {
  UIControlStateNormal = 0,
} UIControlState;

typedef enum {
  UIControlEventTouchUpInside = 1 << 6,
} UIControlEvents;

@interface UIApplication : NSObject
+ (instancetype)sharedApplication;
- (id)delegate;
- (CGRect)statusBarFrame;
- (BOOL)isStatusBarHidden;
- (void)setStatusBarHidden:(BOOL)hidden;
@end
@interface UIScreen : NSObject
+ (instancetype)mainScreen;
- (CGRect)applicationFrame;
@end
@interface UIColor : NSObject
+ (instancetype)colorWithRed:(CGFloat)r
                       green:(CGFloat)g
                        blue:(CGFloat)b
                       alpha:(CGFloat)a;
+ (instancetype)colorWithWhite:(CGFloat)w alpha:(CGFloat)a;
+ (instancetype)clearColor;
+ (instancetype)blackColor;
+ (instancetype)whiteColor;
+ (instancetype)darkGrayColor;
+ (instancetype)grayColor;
+ (instancetype)lightGrayColor;
+ (instancetype)blueColor;
+ (instancetype)brownColor;
+ (instancetype)cyanColor;
+ (instancetype)greenColor;
+ (instancetype)magentaColor;
+ (instancetype)orangeColor;
+ (instancetype)purpleColor;
+ (instancetype)redColor;
+ (instancetype)yellowColor;
@end
@interface UIEvent : NSObject
@end
@class UIView;
@interface UITouch : NSObject
- (CGPoint)locationInView:(UIView *)view;
@end
@interface UIResponder : NSObject
@end
@class UIWindow;
@interface UIView : UIResponder
- (instancetype)initWithFrame:(CGRect)frame;
- (CALayer *)layer;
- (CGRect)bounds;
- (CGRect)frame;
- (void)setBounds:(CGRect)bounds;
- (void)setFrame:(CGRect)frame;
- (void)setTransform:(CGAffineTransform)transform;
- (CGPoint)convertPoint:(CGPoint)point fromView:(UIView *)view;
- (CGPoint)convertPoint:(CGPoint)point toView:(UIView *)view;
- (CGRect)convertRect:(CGRect)point fromView:(UIView *)view;
- (CGRect)convertRect:(CGRect)point toView:(UIView *)view;
- (UIView *)hitTest:(CGPoint)point withEvent:(UIEvent *)event;
- (UIWindow *)window;
- (void)addSubview:(UIView *)view;
- (void)removeFromSuperview;
- (NSArray<UIView *> *)subviews;
- (void)layoutSubviews;
- (void)setBackgroundColor:(UIColor *)color;
- (CGFloat)alpha;
- (void)setAlpha:(CGFloat)alpha;
- (BOOL)isHidden;
- (void)setHidden:(BOOL)hidden;
- (CALayer *)layer;
@end
@interface UIWindow : UIView
- (void)makeKeyAndVisible;
- (CGPoint)convertPoint:(CGPoint)point fromWindow:(UIWindow *)window;
- (CGPoint)convertPoint:(CGPoint)point toWindow:(UIWindow *)window;
- (CGRect)convertRect:(CGRect)point fromWindow:(UIWindow *)window;
- (CGRect)convertRect:(CGRect)point toWindow:(UIWindow *)window;
@end
@interface UILabel : UIView
- (void)setText:(NSString *)text;
- (void)setTextAlignment:(UITextAlignment)alignment;
- (void)setTextColor:(UIColor *)color;
- (void)setNumberOfLines:(NSInteger)lines;
@end
@interface UIImage : NSObject
+ (instancetype)imageWithCGImage:(CGImageRef)cgImage;
@end
@interface UIImageView : UIView
- (void)setImage:(UIImage *)image;
@end
@interface UIControl : UIView
- (void)addTarget:(id)target
              action:(SEL)action
    forControlEvents:(UIControlEvents)events;
@end
@interface UIButton : UIControl
+ (instancetype)buttonWithType:(UIButtonType)type;
- (void)setTitle:(NSString *)title forState:(UIControlState)state;
@end

int UIApplicationMain(int, char **, NSString *, NSString *);

NSString *NSStringFromCGPoint(CGPoint);
NSString *NSStringFromCGSize(CGSize);
NSString *NSStringFromCGRect(CGRect);

void NSLog(NSString *, ...);

#endif // TOUCHHLE_SYSTEM_HEADERS_H
