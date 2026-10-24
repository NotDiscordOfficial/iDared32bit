/*
    SDL_uikit_main.c, placed in the public domain by Sam Lantinga  3/18/2019
*/

/* Include the SDL main definition header */
#include "SDL_main.h"

#if defined(__IPHONEOS__) || defined(__TVOS__)

#ifndef SDL_MAIN_HANDLED
#ifdef main
#undef main
#endif


/* The native app picker: Swift in AppPicker.swift, given to Rust here. */
#include <stddef.h>
#include <stdint.h>
typedef char *(*idared_app_picker_fn)(const char *, const uint8_t *const *, size_t);
extern char *idared_swiftui_app_picker(const char *, const uint8_t *const *, size_t);
extern void idared_set_swiftui_app_picker(idared_app_picker_fn);
/* Gives games opened with "Open in iDared 32bit" to the picker too. */
extern void idared_install_open_in_hook(void);

int
main(int argc, char *argv[])
{
    idared_set_swiftui_app_picker(idared_swiftui_app_picker);
    idared_install_open_in_hook();
    return SDL_UIKitRunApp(argc, argv, SDL_main);
}
#endif /* !SDL_MAIN_HANDLED */

#endif /* __IPHONEOS__ || __TVOS__ */

/* vi: set ts=4 sw=4 expandtab: */
