/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */

#import "system_headers.h"
#include <CoreFoundation/CFArray.h>

int test_CraftedCompatCFArray(void) {
  CFMutableArrayRef array =
      CFArrayCreateMutable(kCFAllocatorDefault, 10, &kCFTypeArrayCallBacks);
  if (array == NULL) {
    return -1;
  }
  CFStringRef first =
      CFStringCreateWithCString(NULL, "first", kCFStringEncodingUTF8);
  CFStringRef second =
      CFStringCreateWithCString(NULL, "second", kCFStringEncodingUTF8);
  CFArrayAppendValue(array, first);
  CFArrayAppendValue(array, second);
  CFRelease(first);
  CFRelease(second);
  if (CFArrayGetCount(array) != 2) {
    CFRelease(array);
    return -2;
  }
  if (CFStringGetLength((CFStringRef)CFArrayGetValueAtIndex(array, 0)) != 5) {
    CFRelease(array);
    return -3;
  }
  CFArrayRemoveValueAtIndex(array, 0);
  if (CFArrayGetCount(array) != 1) {
    CFRelease(array);
    return -4;
  }
  CFRelease(array);

  // NULL callback tables store arbitrary pointer values without retaining.
  CFMutableArrayRef raw = CFArrayCreateMutable(NULL, 3, NULL);
  CFArrayAppendValue(raw, (const void *)0x1234);
  if (CFArrayGetValueAtIndex(raw, 0) != (const void *)0x1234) {
    CFRelease(raw);
    return -5;
  }
  CFArrayRemoveValueAtIndex(raw, 0);
  CFRelease(raw);
  return 0;
}

int test_CraftedCompatNSDataURL(void) {
  NSAutoreleasePool *pool = [[NSAutoreleasePool alloc] init];
  NSURL *custom = [NSURL URLWithString:[NSString stringWithUTF8String:"crafted://missing-resource"]];
  NSData *data = [[NSData alloc] initWithContentsOfURL:custom];
  if (data != nil) {
    [data release];
    [pool drain];
    return -1;
  }
  NSURL *relative = [NSURL URLWithString:[NSString stringWithUTF8String:"crafted_missing_resource.bin"]];
  data = [[NSData alloc] initWithContentsOfURL:relative];
  if (data != nil) {
    [data release];
    [pool drain];
    return -2;
  }
  [pool drain];
  return 0;
}
