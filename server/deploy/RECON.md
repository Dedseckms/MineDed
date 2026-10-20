# Разведка сервера владельца (R9)

- Дата: 2026-10-20, ~09:37 MSK.
- Режим: только чтение, без `sudo`. На сервере ничего не создавалось и не менялось.
- Учётные данные SSH сюда не записываются.

## 1. ОС и ядро

| Параметр | Значение |
|---|---|
| ОС | Ubuntu 24.04.4 LTS (noble) |
| Ядро | 6.8.0-111-generic, x86_64 |
| Виртуализация | `none` (железо, бывший ноутбук) |
| Uptime | 153 дня, load average 0.01 / 0.09 / 0.06 |
| Hostname | `ded-sec-kms` |

## 2. Ресурсы

- **CPU:** AMD A4-4300M APU, 1 ядро / 2 потока, 1.4–2.5 ГГц (2012 г.). AES-NI есть. Очень слабый.
- **RAM:** 5.3 GiB. Занято 1.4 GiB, свободно 225 MiB, buff/cache 4.0 GiB, **available 3.9 GiB**.
- **Swap:** файл `/swap.img`, 4 GiB, занято около 6 MiB.
- **Диск:** `/dev/sda` 238.5 GiB, SSD (`ROTA=0`). `/` = 233G, занято 19G (9%), свободно 203G. Inode занято 3%.
- **Нагрузка в простое:** CPU idle 88–99%, iowait 0.

Память по сервисам (cgroup `MemoryCurrent`, включая page cache):

| Что | Память | Примечание |
|---|---|---|
| Контейнер `python main.py` (бот, cwd `/app`) | ~470 MiB | ~490 MiB RSS; в среднем ~2.6% одного ядра за 153 дня |
| Контейнер postgres (uid 70, alpine) | ~110 MiB | много воркеров |
| Контейнер redis | ~5 MiB | |
| `containerd.service` | ~330 MiB | |
| `docker.service` | ~120 MiB | |
| `caddy.service` (хостовый) | ~73 MiB | |
| `mihomo.service` | ~30 MiB | |

- `docker stats` недоступен: нет прав на `docker.sock` (см. §4).
- Канал (upload/download) не замерялся. **Замер нужен отдельно**, прежде чем планировать трафик relay и mirror-cache.

## 3. Порты (`ss -tulpn` без sudo, имена процессов скрыты)

| Порт | Протокол | Bind | Кто (по косвенным данным) |
|---|---|---|---|
| 22 | tcp | 0.0.0.0, [::] | sshd. Снаружи доступен как **19999** (проброс на роутере) |
| 53 | tcp/udp | 127.0.0.53, 127.0.0.54 | systemd-resolved |
| 80 | tcp | * | **хостовый Caddy** (systemd) |
| 443 | tcp | * | **хостовый Caddy** |
| 443 | udp | * | хостовый Caddy (HTTP/3) |
| 2019 | tcp | 127.0.0.1 | admin API Caddy |
| 7890 | tcp/udp | * | mihomo `mixed-port` |
| 8080 | tcp | 0.0.0.0, [::] | `docker-proxy` → контейнер 172.20.0.4:8080 (python-бот) |
| 9090 | tcp | * | не установлен (нужен `sudo ss`). Вероятно, процесс хоста: docker-proxy для него нет |
| 3478 | udp | — | **свободен** |
| 25565 | tcp | — | свободен (майн-сервер не планируется) |

`ss -s`: 17 TCP-сокетов, 6 установленных соединений, 7 UDP.

## 4. Docker

- `docker.service` и `containerd` активны.
- `/var/run/docker.sock` имеет права `root:docker 660`.
- Пользователь `dedseckms` (`id`: группы adm, sudo, lxd, …) **не входит в группу `docker`**. Поэтому `docker version/ps/stats/network/volume/compose ls` возвращают `permission denied`.
- Что видно через systemd/cgroups и `ip`:
  - 3 запущенных контейнера: python-бот, postgres, redis;
  - сети: `docker0` 172.17.0.0/16 (down), `br-f78a8a3c58c7` 172.19.0.0/16 (down), `br-4e6580ee9ce1` 172.20.0.0/16 (up, сеть бота).
- Образы, тома и compose-проекты не видны. Нужен запрос на одобрение (§10).

## 5. mihomo

- Запуск: **systemd** `mihomo.service`.
  - `/usr/local/bin/mihomo -d /etc/mihomo`, `User=root`, `Restart=on-failure`.
  - Это не контейнер.
- Режимные поля конфига (`/etc/mihomo/config.yaml`, читается без sudo):
  - `mixed-port: 7890`;
  - `allow-lan: true`;
  - `mode: rule`;
  - `log-level: info`;
  - `external-controller: ""` (контроллер выключен);
  - `dns.enable: true`, `dns.listen: 0.0.0.0:53`, `dns.enhanced-mode: fake-ip`.
- **Секции `tun:` в конфиге нет.** TUN-интерфейса нет: в `ip -br link` только eno1, wlo1, docker-мосты и veth.
- `ip rule` стандартный (local, main, default). Дополнительных таблиц маршрутизации нет. Default route идёт через `eno1` на 192.168.31.1.
  - Вывод: mihomo **не перехватывает трафик хоста**. Он работает как прокси на 7890 для тех, кто сам на него указывает.
- DNS mihomo на 0.0.0.0:53 фактически **не слушает**: порт 53 занят systemd-resolved на 127.0.0.53/54. Скорее всего, bind не удался. На наши сервисы это не влияет, но владельцу стоит знать.
- 7890 снаружи (с интернета) закрыт, роутер его не пробрасывает. С `allow-lan: true` порт открыт для LAN 192.168.31.0/24.

## 6. Reverse proxy

- **Caddy 2.6.2 на хосте** (systemd `caddy.service`, `/etc/caddy/Caddyfile`, пользователь `caddy`). Держит 80/tcp, 443/tcp и 443/udp.
- nginx, traefik и haproxy не запущены.
- Обслуживаемые домены (все проксируются на `localhost:8080`, то есть на бота):
  - `hooks.bot.otakuro.ru`;
  - `cabinet.bot.otakuro.ru` (`/api/*` → 8080, остальное отдаётся статикой из `/srv/cabinet`);
  - `app.bot.otakuro.ru`;
  - `do-did-done.net.otakuro.ru`.
- Для `dedseckms.ru` блока сайта в Caddyfile нет.

## 7. Белый IP или NAT

- Адрес хоста: `eno1` = 192.168.31.83/24 (DHCP), шлюз 192.168.31.1 (домашний роутер). Глобального IPv6 нет.
- Внешний IP: `curl api.ipify.org` → **62.182.138.63**. Через `--interface eno1` тот же адрес, TUN нет.
- A-записи `dedseckms.ru` и `*.otakuro.ru` → 62.182.138.63, совпадают с внешним IP.
- Проверка снаружи (TCP с другой сети, 151.241.x.x):

| Порт | Результат |
|---|---|
| 80/tcp | открыт |
| 443/tcp | открыт (HTTPS-сайт бота отвечает) |
| 19999/tcp | открыт (SSH) |
| 8080, 9090, 7890, 3478/tcp | закрыты |

- **Вывод:** белый IP на роутере, **NAT с пробросом портов** (не CGNAT). 443/tcp уже проброшен на этот хост.
- UDP снаружи не проверялся: на 3478 нет слушателя, а UDP-проброс без ответа сервиса не подтвердить. Владелец открывает на роутере **3478/udp** и, если нужен HTTP/3, **443/udp** → 192.168.31.83. После деплоя relay проверяем повторно.
- IP получен по DHCP. Нужно закрепить на роутере (DHCP-резервация), иначе пробросы сломаются.
- Нужно уточнить у владельца: статичен ли внешний IP у провайдера.

## 8. Каталоги и права

- `/opt/skinserver`, `/opt/relay` **не существуют**. В `/opt` только `containerd` (root).
- `id` = uid 1000 `dedseckms`, группы adm, cdrom, sudo, audio, dip, plugdev, lxd. Группы `docker` нет.
- ufw: `ENABLED=no`, сервис `ufw` в состоянии active (правил нет). `nftables.service` inactive. Правила iptables от Docker без sudo не прочитать.

## 9. Канал

Не замерялся. Нужен отдельный замер (iperf3 или speedtest) с согласия владельца, особенно upload: relay и mirror-cache отдают трафик наружу.

## 10. Выводы

### Бюджет ресурсов

Из 3.9 GiB available оставляем запас владельцу (бот, postgres, page cache). Нашему стеку — **не более ~512 MiB RAM суммарно** через `mem_limit` и **не более ~1 CPU** через `cpus`.

| Сервис | RAM (limit) | CPU (limit) | Диск |
|---|---|---|---|
| Drasl | 128 MiB | 0.5 | < 1 GiB (sqlite + текстуры) |
| hub (axum) | 64 MiB | 0.5 | ~0 |
| iroh-relay | 128 MiB | 0.75 | ~0 |
| mirror-cache (nginx) | 64 MiB | 0.5 | `max_size=20g` (диска хватает, SSD) |
| **Итого** | **~384–512 MiB** | ≤ 1.0 в среднем | ~21 GiB |

- CPU — главное узкое место: 1 физическое ядро 2012 года. iroh-relay используется только как fallback, когда hole punching не удался. При заметном relay-трафике TLS и пересылка будут грузить CPU. Нужен лимит `cpus` и мониторинг. При росте relay стоит вынести на VPS.
- Майн-сервер на хосте не ставим (P2P у игроков).

### Что ставить, а что нет

- **Ставить:**
  - Drasl;
  - hub;
  - iroh-relay;
  - mirror-cache (nginx, лимиты и `max_size` обязательны).
- **Не ставить:** контейнер `caddy` из текущего `server/docker-compose.yml`. Порты 80/443 заняты хостовым Caddy с сайтами владельца, второй прокси на них не встанет.
- Вместо него добавляем свои site-блоки в существующий хостовый Caddy:
  - `skins.<домен>`, `hub.<домен>`, `relay.<домен>`, `mirror.<домен>`;
  - `reverse_proxy` на `127.0.0.1:<порт>`.
  - Для этого нужен `sudo` (см. ниже). Альтернатива — `import /opt/skinserver/caddy/*.caddy` одной строкой, тогда дальше правим только свои файлы.
- Учесть: Caddy 2.6.2 старый. Websocket и HTTP/2 через `reverse_proxy` для relay работают, но версию обновляет владелец, не мы.

### Конфликты портов и обход

| Наш сервис | Конфликт | Решение |
|---|---|---|
| caddy 80/443 | занято хостовым Caddy | убрать из compose, использовать хостовый Caddy |
| hub 8080 | **8080 занят ботом** (0.0.0.0) | публиковать hub на `127.0.0.1:18080` (или любой свободный), внутри контейнера можно оставить 8080 |
| drasl 25585 | нет | `127.0.0.1:25585` |
| iroh-relay HTTPS | 443 у Caddy | relay слушает `127.0.0.1:<порт>`, Caddy проксирует `relay.<домен>` (HTTP upgrade / websocket) |
| iroh-relay 3478/udp (STUN/QAD) | свободно | публиковать `3478:3478/udp` на хосте + проброс на роутере. Сверить порт QAD с выбранной версией iroh-relay при закреплении тега |
| mirror-cache | нет | `127.0.0.1:<порт>` за Caddy |

Все наши порты, кроме UDP-порта relay, публикуем только на `127.0.0.1`. Наружу выходит только хостовый Caddy.

### Риски для mihomo и чужих контейнеров

- mihomo без TUN и без своих маршрутов, поэтому наш Docker-трафик он не перехватывает. Маршруты, DNS и 7890 не трогаем.
- Если владелец когда-нибудь включит в mihomo TUN с `auto-route`, исходящий трафик Drasl (fallback на Mojang) и mirror-cache пойдёт через прокси. Это надо учитывать.
- Нельзя занимать порт 53 (mihomo DNS настроен на 0.0.0.0:53).
- Хостовый Caddy общий с сайтами бота. Ошибка в Caddyfile при reload уронит конфиг всех сайтов. Поэтому:
  - перед правкой — бэкап;
  - `caddy validate`;
  - только `systemctl reload caddy`, не restart.
  - Предпочтительный вариант — отдельный импортируемый файл.
- Docker-сеть нашего compose создаётся отдельно. Подсети 172.17, 172.19 и 172.20 заняты, Docker сам выберет свободную. Чужие сети и тома не трогаем, `prune` не делаем.
- Память: лимиты обязательны. Без них mirror-cache или relay под нагрузкой могут вытеснить бота и postgres в swap.
- Внешний IP через DHCP и NAT: при смене IP или переназначении локального адреса всё перестанет работать вместе с сайтами владельца.

### Запросы на одобрение (sudo), передать владельцу через lead

1. Чтение Docker-инвентаря (только чтение):
   ```
   sudo docker version --format '{{.Server.Version}}'
   sudo docker ps -a --format 'table {{.Names}}\t{{.Image}}\t{{.Ports}}\t{{.Status}}'
   sudo docker stats --no-stream
   sudo docker network ls; sudo docker volume ls; sudo docker compose ls
   ```
2. Кто слушает 9090 и прочие порты (только чтение): `sudo ss -tulpn`.
3. Правила iptables Docker и NAT (только чтение): `sudo iptables -S; sudo iptables -t nat -S`.
4. Каталоги для деплоя (запись):
   ```
   sudo mkdir -p /opt/skinserver /opt/relay
   sudo chown dedseckms:dedseckms /opt/skinserver /opt/relay
   ```
5. Доступ к Docker для деплоя, на выбор владельца:
   - (a) каждую команду `docker compose` выполнять через `sudo` с одобрением;
   - (b) `sudo usermod -aG docker dedseckms` (фактически root-доступ).
6. Позже, при деплое: в `/etc/caddy/Caddyfile` добавить строку `import /opt/skinserver/caddy/*.caddy`, затем выполнить
   ```
   sudo cp /etc/caddy/Caddyfile /etc/caddy/Caddyfile.bak-<дата>
   sudo caddy validate --config /etc/caddy/Caddyfile
   sudo systemctl reload caddy
   ```
7. **Владельцу на роутере** (не sudo):
   - проброс 3478/udp (и 443/udp при желании) → 192.168.31.83;
   - DHCP-резервация 192.168.31.83;
   - DNS-записи поддоменов для наших сервисов.
8. Отдельный замер канала (iperf3 или speedtest): по согласованию.
