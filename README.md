# Docker Image Patcher

Docker 镜像增量修补工具.

当镜像有变动时, 如果构建镜像所使用的基镜像(FROM)没有更改, 则可以使用本工具生成增量补丁, 减少更新包大小.

## 用法

假设旧镜像为 `service:1.0.0`, 并且已经导入到现场环境中. 在研发环境构建的新镜像为 `service:1.0.1`, 则按如下步骤进行操作:

### 导出旧镜像层信息

在现场环境中, 执行:

```bash
docker image inspect service:1.0.0 > inspect__1.0.0.json
```

并将生成的 `inspect__1.0.0.json` 传回研发环境

### 生成增量补丁

在研发环境中, 执行:

```bash
docker save service:1.0.1 > service__1.0.1.tar
```

导出新镜像 `service__1.0.1.tar`

执行:

```bash
docker-image-patcher delta service__1.0.1.tar inspect__1.0.0.json
```

生成增量补丁 `service__1.0.1.delta`, 并传至现场环境

### 修补旧镜像

在现场环境中, 执行:

```bash
docker save service:1.0.0 > service__1.0.0.tar
```

导出旧镜像 `service__1.0.0.tar`, 然后执行:

```bash
docker-image-patcher patch service__1.0.0.tar service__1.0.1.delta
```

生成修补后的镜像 `service__1.0.1.tar`

执行命令导入新镜像:

```bash
docker load < service__1.0.1.tar
```