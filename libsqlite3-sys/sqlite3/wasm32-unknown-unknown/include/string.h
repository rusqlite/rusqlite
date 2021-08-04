#include <stddef.h>

int strcmp(const char *s1, const char *s2);
size_t strcspn(const char *s1, const char *s2);
size_t strlen(const char *str);
int strncmp(const char *s1, const char *s2, size_t n);
char *strrchr(const char *p, int ch);

int memcmp(const void *str1, const void *str2, size_t n);
void *memcpy(void *dest, const void *src, size_t n);
void *memmove(void *str1, const void *str2, size_t n);
void *memset(void *str, int c, size_t n);

#define DEF_STRONG(x)
#define __weak_alias(x, y)
