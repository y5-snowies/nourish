#include <fcntl.h>
#include <stdio.h>
#include <gbm.h>
#include <unistd.h>
int main(int argc, char**argv){
  const char*p = argc>1?argv[1]:"/dev/dri/renderD128";
  fprintf(stderr,"open %s...\n",p); fflush(stderr);
  int fd=open(p,O_RDWR|O_CLOEXEC); fprintf(stderr,"fd=%d\n",fd); fflush(stderr);
  if(fd<0) return 1;
  fprintf(stderr,"gbm_create_device...\n"); fflush(stderr);
  struct gbm_device*g=gbm_create_device(fd); fprintf(stderr,"gbm=%p backend=%s\n",(void*)g, g?gbm_device_get_backend_name(g):"-"); fflush(stderr);
  if(!g) return 2;
  fprintf(stderr,"gbm_bo_create 64x64 XR24 linear...\n"); fflush(stderr);
  struct gbm_bo*bo=gbm_bo_create(g,64,64,GBM_FORMAT_XRGB8888,GBM_BO_USE_RENDERING|GBM_BO_USE_LINEAR);
  fprintf(stderr,"bo=%p\n",(void*)bo); fflush(stderr);
  if(!bo) return 3;
  fprintf(stderr,"stride=%u mod=0x%llx fd...\n",gbm_bo_get_stride(bo),(unsigned long long)gbm_bo_get_modifier(bo)); fflush(stderr);
  int bfd=gbm_bo_get_fd(bo); fprintf(stderr,"bo fd=%d\n",bfd); fflush(stderr);
  return 0;
}
