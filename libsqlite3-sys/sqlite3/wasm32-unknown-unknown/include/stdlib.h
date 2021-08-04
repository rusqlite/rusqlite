#include <stddef.h>

void *malloc(size_t size);
void free(void *ptr);
void *realloc(void *ptr, size_t size);

void qsort(void *a, size_t n, size_t es, int (*cmp)(const void *, const void *));

#define DEF_STRONG(x)
