# Aire Control

Programa para Windows que controla un aire acondicionado Carrier desde la bandeja del sistema y guarda un
historial de sus datos. Está hecho en Rust, sin navegador integrado, para gastar lo mínimo.

## Cómo instalarlo

1. Bajá `AireControl.exe` de la sección **Releases** de este repositorio y guardalo en una carpeta fija.
2. Abrilo. Como el programa no está firmado, Windows puede mostrar "Windows protegió su PC": tocá **Más información**
   y después **Ejecutar de todos modos**.
3. Entrá con el email y la contraseña de la app de Carrier. El aire tiene que estar vinculado a esa cuenta.

Hay que usar una cuenta propia: Midea permite una sola sesión a la vez, y dos copias del programa con la misma cuenta
se van sacando entre sí.

## Cómo usarlo

Aparece un ícono abajo a la derecha con la temperatura del cuarto. Si Windows lo esconde, está en la flecha de íconos
ocultos, y desde ahí podés arrastrarlo a la barra de tareas.

- **Clic izquierdo:** abre o cierra el panel.
- **Clic derecho:** prender o apagar, sumar tiempo, iniciar con Windows y salir.
- **Regla del panel:** arrastrala, o usá la rueda del ratón o las flechas ↑ y ↓ para cambiar la temperatura.
- **Esc o clic afuera:** cierra el panel. El programa sigue registrando datos.

Si la cuenta tiene más de un aire, se elige en el engranaje del panel. El historial es uno solo, así que al cambiar de
aire se mezclan los datos de los dos.

## Temporizadores

Cada toque suma 30 minutos: tres toques son 90.

- **Mantener prendido:** prende el aire si estaba apagado y lo apaga cuando termina el tiempo.
- **Prender más tarde:** espera y después prende. Si además tocás "Mantener prendido", el aire queda prendido
  ese tiempo a partir del encendido.

Los temporizadores los cumple este programa, no el aire:

- La PC tiene que estar prendida y con internet. Si estaba apagada, el apagado se cumple al volver a abrir el
  programa. Un encendido atrasado más de 15 minutos no se hace: el programa avisa.
- Mientras corre un temporizador, Windows no suspende la PC, aunque la pantalla sí puede apagarse.
- Apagar el aire a mano, incluso con el control remoto, cancela el temporizador de apagado.

## Qué datos guarda

Se guardan en `%LOCALAPPDATA%\AireControl\aire.db`, y desde el panel se pueden exportar a CSV.

- **Cada lectura:** encendido, modo, objetivo, temperatura adentro y afuera, ventilador, oscilación, eco, turbo,
  sueño, pantalla, aviso de filtro y código de error.
- **Cada evento:** qué cambió y desde dónde (la app, el temporizador, o el control remoto u otra app).
- **Calculado:** tiempo prendido, cantidad de encendidos, tiempo por modo, promedios, mínimas y máximas.

El consumo en kWh es una **estimación**: multiplica el tiempo prendido, sin contar Ventilar, por la potencia
configurada en los ajustes (1100 W si no la cambiás). No sabe cuándo descansa el compresor, así que puede dar de más.
El aire con el que se probó no respondió a las consultas de consumo real ni de compresor.

Las lecturas se hacen cada minuto con el aire prendido, cada 5 minutos con el aire apagado y cada 20 segundos con
el panel abierto.

## Cómo funciona

Los comandos viajan por la nube de Midea, igual que en la app, porque el servidor ya no entrega la clave para hablarle
al aire directamente por la red local. Por eso:

- Hacen falta internet y los servidores de Midea funcionando.
- El programa puede dejar de andar si Midea cambia ese servicio.

La contraseña queda en el Administrador de credenciales de Windows. "Cerrar sesión" la borra y cancela los temporizadores.

Se probó en Windows 11. En Windows 10 debería funcionar sin el fondo translúcido, pero no se probó.

## Compilar

Hace falta Rust con las herramientas de compilación de Visual Studio.

```bash
cargo build --release
```

```bash
cargo test
```

Para ver el panel con un aire simulado, sin tocar el aire real ni usar tu cuenta:

```bash
cargo run --features demo -- --demo
```

## Origen del protocolo

Las tramas del aire son un puerto de [msmart-ng](https://github.com/mill1000/midea-msmart), y las pruebas comparan el resultado
byte a byte con esa librería. La forma de reenviar comandos por la nube está basada en lo que documenta
[jasonzzeng/midea-streamdeck](https://github.com/jasonzzeng/midea-streamdeck), que no incluye una licencia.
