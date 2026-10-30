# Установка MineDed

**Русский** · [English](#english)

Скачивайте лаунчер только с [сайта](https://mineded.dedseckms.ru/) или со страницы
[Releases на GitHub](https://github.com/Dedseckms/MineDed/releases/latest). Копии из других мест могут быть изменены.

## Windows 10 / 11

1. Скачайте **`MineDed_x.y.z_x64-setup.exe`** (рекомендуется: ставится для текущего пользователя, права
   администратора не нужны) или `.msi` (для установки администратором).
2. **Браузер может предупредить, что файл скачивают редко.** В Chrome/Edge нажмите «⋯» у загрузки →
   «Сохранить» / «Всё равно сохранить».
3. **«Windows защитила ваш компьютер» (SmartScreen).** Нажмите **«Подробнее»** → **«Выполнить в любом случае»**.
   Предупреждение появляется потому, что у установщика пока нет платной цифровой подписи и мало скачиваний —
   это не значит, что в нём вирус. Исходный код открыт, установщики собирает GitHub Actions.
4. Пройдите установщик. Ярлык появится на рабочем столе и в меню «Пуск».
5. **Брандмауэр Windows** при первой игре с друзьями спросит про доступ к сети для MineDed и Java —
   разрешите для **частных сетей**, иначе друзья не смогут подключиться.

## Linux

### AppImage (любой дистрибутив)

```sh
chmod +x MineDed_*_amd64.AppImage
./MineDed_*_amd64.AppImage
```

Если не запускается с ошибкой про FUSE, установите `libfuse2`:

| Дистрибутив | Команда |
|---|---|
| Ubuntu 22.04, Debian, Mint | `sudo apt install libfuse2` |
| Ubuntu 24.04+ | `sudo apt install libfuse2t64` |
| Arch, CachyOS, Manjaro | `sudo pacman -S fuse2` |
| Fedora | `sudo dnf install fuse-libs` |

AppImage обновляется сам.

### .deb (Ubuntu, Debian, Mint)

```sh
sudo apt install ./MineDed_*_amd64.deb
```

Новые версии ставятся так же — скачайте свежий `.deb` и повторите команду.

### Если окно лаунчера чёрное

Бывает на старых видеокартах. С версии 1.0.6 лаунчер обходит это сам; если не помогло, запустите так:

```sh
WEBKIT_DISABLE_DMABUF_RENDERER=1 WEBKIT_DISABLE_COMPOSITING_MODE=1 ./MineDed_*_amd64.AppImage
```

## Первый запуск

1. **Аккаунты** → оффлайн-аккаунт (просто ник) или **«Мой сервер»** — вход/регистрация по инвайту, со скинами.
2. **Сборки** → «Создать»: версия и загрузчик. Java скачается сама.
3. **Играть.** Для слабых ПК выберите пресет оптимизации в настройках сборки.

## Обновления

Лаунчер проверяет обновления при запуске и ставит их по кнопке. Обновления подписаны: подменённое
обновление лаунчер не установит.

## Где лежат данные

| | Windows | Linux |
|---|---|---|
| Сборки, настройки, кэш | `%APPDATA%\ru.dedseckms.mineup` | `~/.local/share/ru.dedseckms.mineup` |
| Логи лаунчера | `%LOCALAPPDATA%\ru.dedseckms.mineup\logs` | `~/.local/share/ru.dedseckms.mineup/logs` |

Лог игры — во вкладке «Играть» → «Лог игры». Его стоит прикладывать к сообщению об ошибке.

## Удаление

- **Windows:** Параметры → Приложения → MineDed → Удалить. Сборки и миры остаются в папке данных —
  удалите её вручную, если они не нужны.
- **Linux:** удалите файл AppImage; для `.deb` — `sudo apt remove <пакет>` (имя пакета покажет `dpkg -l | grep -i mine`). Данные — в папке выше.

## Проблемы

- Ошибка или вопрос → [issue на GitHub](https://github.com/Dedseckms/MineDed/issues/new/choose)
  или [Telegram-канал](https://t.me/minededlaunch).
- Уязвимость → приватно, см. [SECURITY.md](../SECURITY.md).

---

## English

Download only from the [website](https://mineded.dedseckms.ru/) or
[GitHub Releases](https://github.com/Dedseckms/MineDed/releases/latest).

**Windows:** run `MineDed_x.y.z_x64-setup.exe` (no admin rights) or the `.msi`. If the browser says the file is
rarely downloaded, choose "Keep". If SmartScreen shows "Windows protected your PC", click **More info → Run anyway**
(the installer isn't code-signed yet; the source is open and builds come from GitHub Actions). Allow MineDed and
Java on **private networks** when the firewall asks, so friends can join your world.

**Linux:** `chmod +x MineDed_*_amd64.AppImage && ./MineDed_*_amd64.AppImage` (install `libfuse2` /
`libfuse2t64` / `fuse2` if FUSE is missing), or `sudo apt install ./MineDed_*_amd64.deb`. Black window on old GPUs:
start with `WEBKIT_DISABLE_DMABUF_RENDERER=1 WEBKIT_DISABLE_COMPOSITING_MODE=1`.

**Data:** `%APPDATA%\ru.dedseckms.mineup` (Windows), `~/.local/share/ru.dedseckms.mineup` (Linux); logs in the
`logs` subfolder (Windows: under `%LOCALAPPDATA%`). Updates are signed and installed from inside the launcher.
