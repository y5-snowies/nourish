#include <fcntl.h>
#include <stdio.h>
#include <gbm.h>
#include <unistd.h>
static void try_flags(struct gbm_device*g,const char*label,uint32_t flags){
  struct gbm_bo*bo=gbm_bo_create(g,64,64,GBM_FORMAT_XRGB8888,flags);
  if(!bo){printf("%-28s -> NULL\n",label);return;}
  int fd=gbm_bo_get_fd(bo);
  printf("%-28s -> ok stride=%u mod=0x%016llx fd=%d\n",label,gbm_bo_get_stride(bo),
         (unsigned long long)gbm_bo_get_modifier(bo),fd);
  if(fd>=0)close(fd); gbm_bo_destroy(bo);
}
int main(void){
  int fd=open("/dev/dri/renderD128",O_RDWR|O_CLOEXEC); struct gbm_device*g=gbm_create_device(fd);
  try_flags(g,"RENDERING",GBM_BO_USE_RENDERING);
  try_flags(g,"RENDERING|LINEAR",GBM_BO_USE_RENDERING|GBM_BO_USE_LINEAR);
  try_flags(g,"RENDERING|WRITE",GBM_BO_USE_RENDERING|GBM_BO_USE_WRITE);
  try_flags(g,"WRITE",GBM_BO_USE_WRITE);
  try_flags(g,"WRITE|LINEAR",GBM_BO_USE_WRITE|GBM_BO_USE_LINEAR);
  try_flags(g,"SCANOUT|RENDERING",GBM_BO_USE_SCANOUT|GBM_BO_USE_RENDERING);
  uint64_t lin=0;
  struct gbm_bo*bo=gbm_bo_create_with_modifiers(g,64,64,GBM_FORMAT_XRGB8888,&lin,1);
  printf("%-28s -> %s","with_modifiers[LINEAR]",bo?"ok":"NULL");
  if(bo){int f=gbm_bo_get_fd(bo);printf(" stride=%u mod=0x%016llx fd=%d",gbm_bo_get_stride(bo),(unsigned long long)gbm_bo_get_modifier(bo),f);if(f>=0)close(f);}
  printf("\n");
  return 0;
}
